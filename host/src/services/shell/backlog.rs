use super::*;
use std::{collections::VecDeque, time::Instant};
struct State {
    items: VecDeque<(u64, Instant, Value)>,
    seq: u64,
    ready: Option<u64>,
    closed: u64,
}
pub(super) struct Backlog {
    ctx: Arc<ServiceContext>,
    state: Mutex<State>,
    boot: String,
}
impl Backlog {
    #[cfg(test)]
    pub(super) fn age(&self, seconds: u64) {
        let mut state = self.state.lock().unwrap();
        for (_, at, _) in &mut state.items {
            *at -= Duration::from_secs(seconds);
        }
    }
    pub fn new(ctx: Arc<ServiceContext>) -> Arc<Self> {
        let svc = Arc::new(Self {
            ctx: ctx.clone(),
            state: Mutex::new(State {
                items: VecDeque::new(),
                seq: 0,
                ready: None,
                closed: 0,
            }),
            boot: glib::uuid_string_random().replace('-', "")[..12].into(),
        });
        let weak = Arc::downgrade(&svc);
        ctx.events.on(
            "app.navigate",
            Arc::new(move |data| {
                if let Some(svc) = weak.upgrade() {
                    svc.navigate(data);
                }
            }),
        );
        let weak = Arc::downgrade(&svc);
        ctx.events.on(
            "shell.appClosed",
            Arc::new(move |data| {
                if let Some(svc) = weak.upgrade() {
                    svc.closed(data);
                }
            }),
        );
        svc
    }
    fn navigate(&self, data: &Value) {
        if data["app"] != "explorer" {
            return;
        }
        let params = if data["params"].is_object() {
            data["params"].clone()
        } else {
            json!({})
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.ready == Some(state.closed) {
            return;
        }
        state.seq += 1;
        let seq = state.seq;
        state.items.push_back((seq, Instant::now(), params));
        while state.items.len() > 16 {
            state.items.pop_front();
        }
    }
    fn closed(&self, data: &Value) {
        if data["app"] == "explorer" {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed += 1;
            state.items.clear();
        }
    }
    pub fn ready(&self, args: &Value) -> ApiResult {
        let page = args["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.chars().count() <= 64)
            .ok_or_else(|| bad_args(self.ctx.lang()))?;
        let payload = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            while state
                .items
                .front()
                .is_some_and(|(_, at, _)| at.elapsed() > Duration::from_secs(30))
            {
                state.items.pop_front();
            }
            let ack = args["ack"]
                .as_i64()
                .map(|n| n as i128)
                .or_else(|| args["ack"].as_u64().map(|n| n as i128));
            if let Some(ack) = ack {
                while state
                    .items
                    .front()
                    .is_some_and(|(seq, _, _)| (*seq as i128) <= ack)
                {
                    state.items.pop_front();
                }
                if state.items.is_empty() {
                    state.ready = Some(state.closed);
                    return Ok(json!({}));
                }
            }
            let seq = state
                .items
                .back()
                .map(|(seq, _, _)| *seq)
                .unwrap_or(state.seq);
            json!({"id":page,"boot":self.boot,"seq":seq,"navigate":state.items.iter().map(|(seq,_,params)|json!({"seq":seq,"params":params})).collect::<Vec<_>>()})
        };
        self.ctx.emit("explorer.backlog", payload);
        Ok(json!({}))
    }
}
