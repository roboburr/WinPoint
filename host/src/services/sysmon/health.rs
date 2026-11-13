use super::{
    ServiceContext,
    common::*,
    performance::{self, Gpu},
    process::ProcessState,
};
use serde_json::{Value, json};
use std::sync::Arc;
fn action(lang: &str, label: &str, page: &str) -> Value {
    json!({"label":tr(lang,label,&[]),"app":"taskmgr","params":{"page":page}})
}
fn item(id: &str, severity: &str, title: String, message: String, action: Value) -> Value {
    json!({"id":id,"severity":severity,"title":title,"message":message,"action":action})
}
/// The processes busy for `seconds` ([`ProcessState::high_cpu`]), named in `lang`. Their names come from the last
/// process list; when that list was built in the other language, build it again, so no name is shown in the old
/// language (round2-gate §2). Before the first list (names from /proc, as in 3b898e6) and while nothing is busy
/// this costs nothing extra (review n3).
pub(super) fn busy(
    ctx: &Arc<ServiceContext>,
    pm: &mut ProcessState,
    lang: &str,
    seconds: f64,
) -> Vec<Value> {
    let hot = pm.high_cpu(seconds);
    if hot.is_empty() || pm.names_lang().is_empty() || pm.names_lang() == lang {
        return hot;
    }
    pm.processes(ctx, false);
    pm.high_cpu(seconds)
}
pub(super) fn health(ctx: &Arc<ServiceContext>, pm: &mut ProcessState, gpu: &Arc<Gpu>) -> Value {
    let lang = ctx.language();
    let t = |key: &str, args: &[(&str, String)]| tr(&lang, key, args);
    let bytes = |n: f64| fmt_bytes(&lang, Some(n));
    let mut items = vec![];
    let volumes = ctx.call("storage.volumes", &json!({})).unwrap_or(json!([]));
    for v in volumes.as_array().into_iter().flatten() {
        let total = v["total"].as_f64().unwrap_or(0.);
        let free = v["free"].as_f64().unwrap_or(0.);
        let used = v["used"].as_f64().unwrap_or(0.);
        if total <= 0. || truth(&v["readOnly"]) || v["kind"] == "optical" || truth(&v["hidden"]) {
            continue;
        }
        let pct = used * 100. / (used + free).max(1.);
        if pct >= 90. {
            let critical = pct >= 97. || truth(&v["isSystem"]) && free < 2. * 1024f64.powi(3);
            let mut a = action(
                &lang,
                if truth(&v["isSystem"]) || truth(&v["isHome"]) {
                    "a_free"
                } else {
                    "a_storage"
                },
                "storage",
            );
            a["params"]["volume"] = v["id"].clone();
            if truth(&v["isSystem"]) || truth(&v["isHome"]) {
                a["params"]["section"] = json!("cleanup");
                a["params"]["tab"] = json!("cleanup")
            }
            let name = v["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| v["mount"].as_str())
                .unwrap_or("");
            items.push(item(
                &format!("disk:{}", v["id"].as_str().unwrap_or("None")),
                if critical { "critical" } else { "warning" },
                t(
                    if critical {
                        "h_disk_crit_t"
                    } else {
                        "h_disk_warn_t"
                    },
                    &[("name", name.into())],
                ),
                t(
                    if critical {
                        "h_disk_crit_m"
                    } else {
                        "h_disk_warn_m"
                    },
                    &[
                        ("free", bytes(free)),
                        ("total", bytes(total)),
                        ("pct", (pct.round_ties_even() as i64).to_string()),
                    ],
                ),
                a,
            ));
        }
    }
    if items.is_empty() {
        items.push(item(
            "disks",
            "ok",
            t("h_disk_ok_t", &[]),
            t("h_disk_ok_m", &[]),
            Value::Null,
        ));
    }
    let mem = performance::meminfo();
    let m = |k: &str| mem.get(k).copied().unwrap_or(0) as f64;
    let total = m("MemTotal");
    let avail = m("MemAvailable");
    let pressure = read("/proc/pressure/memory");
    let psi = pressure
        .lines()
        .find(|l| l.starts_with("some "))
        .and_then(|l| {
            l.split_whitespace()
                .find_map(|f| f.strip_prefix("avg60=").and_then(num))
        });
    let fraction = if total > 0. { avail / total } else { 1. };
    let mut mem_action = action(&lang, "a_procs", "processes");
    mem_action["params"]["sort"] = json!("memory");
    if fraction < 0.05 || psi.is_some_and(|v| v > 30.) {
        items.push(item(
            "memory",
            "critical",
            t("h_mem_crit_t", &[]),
            t("h_mem_crit_m", &[("avail", bytes(avail))]),
            mem_action.clone(),
        ));
    } else if fraction < 0.12 || psi.is_some_and(|v| v > 10.) {
        items.push(item(
            "memory",
            "warning",
            t("h_mem_warn_t", &[]),
            t(
                "h_mem_warn_m",
                &[("avail", bytes(avail)), ("total", bytes(total))],
            ),
            mem_action.clone(),
        ));
    } else {
        items.push(item(
            "memory",
            "ok",
            t("h_mem_ok_t", &[]),
            t(
                "h_mem_ok_m",
                &[("avail", bytes(avail)), ("total", bytes(total))],
            ),
            Value::Null,
        ));
    }
    let swap = m("SwapTotal");
    let swap_used = (swap - m("SwapFree")).max(0.);
    if swap > 0. && swap_used / swap >= 0.5 && swap_used > 256. * 1024f64.powi(2) {
        items.push(item(
            "swap",
            if swap_used / swap >= 0.8 {
                "warning"
            } else {
                "info"
            },
            t("h_swap_warn_t", &[]),
            t(
                "h_swap_warn_m",
                &[("used", bytes(swap_used)), ("total", bytes(swap))],
            ),
            mem_action,
        ));
    } else {
        items.push(item(
            "swap",
            "ok",
            t("h_swap_ok_t", &[]),
            t(
                if swap > 0. {
                    "h_swap_ok_m"
                } else {
                    "h_swap_none_m"
                },
                &[],
            ),
            Value::Null,
        ));
    }
    let hot = busy(ctx, pm, &lang, 30.);
    if hot.is_empty() {
        items.push(item(
            "cpu",
            "ok",
            t("h_cpu_ok_t", &[]),
            t("h_cpu_ok_m", &[]),
            Value::Null,
        ));
    } else {
        for h in hot.iter().take(3) {
            let pid = h["pid"].as_u64().unwrap_or(0);
            let cpu = h["cpu"].as_f64().unwrap_or(0.);
            let mut a = action(&lang, "a_view", "processes");
            a["params"]["pid"] = json!(pid);
            let mut v = item(
                &format!("cpu:{pid}"),
                if cpu >= 50. { "warning" } else { "info" },
                t(
                    "h_cpu_warn_t",
                    &[("name", h["name"].as_str().unwrap_or("").into())],
                ),
                t(
                    "h_cpu_warn_m",
                    &[(
                        "dur",
                        fmt_duration(&lang, h["since"].as_f64().unwrap_or(0.)),
                    )],
                ),
                a,
            );
            v["pid"] = json!(pid);
            v["cpu"] = json!(round(cpu, 1));
            items.push(v);
        }
    }
    let temp = performance::cpu_temperature();
    let mut temp_items = vec![];
    let perf_action = action(&lang, "a_perf", "performance");
    if let Some(temp) = temp.filter(|t| *t >= 85.) {
        let critical = temp >= 95.;
        temp_items.push(item(
            "temp:cpu",
            if critical { "critical" } else { "warning" },
            t(
                if critical {
                    "h_temp_crit_t"
                } else {
                    "h_temp_warn_t"
                },
                &[("what", t("cpu_word", &[]))],
            ),
            t(
                if critical {
                    "h_temp_crit_m"
                } else {
                    "h_temp_warn_m"
                },
                &[("temp", (temp as i64).to_string())],
            ),
            perf_action.clone(),
        ));
    }
    for g in gpu.nvidia(10., 1.) {
        if let Some(temp) = g["temperature"].as_f64().filter(|t| *t >= 85.) {
            let critical = temp >= 92.;
            temp_items.push(item(
                "temp:gpu",
                if critical { "critical" } else { "warning" },
                t(
                    if critical {
                        "h_temp_crit_t"
                    } else {
                        "h_temp_warn_t"
                    },
                    &[("what", t("gpu_word", &[]))],
                ),
                t(
                    if critical {
                        "h_temp_crit_m"
                    } else {
                        "h_temp_warn_m"
                    },
                    &[("temp", (temp as i64).to_string())],
                ),
                perf_action.clone(),
            ));
        }
    }
    if temp_items.is_empty() {
        let mut v = item(
            "temperature",
            "ok",
            t("h_temp_ok_t", &[]),
            if let Some(temp) = temp {
                t("h_temp_ok_m", &[("temp", (temp as i64).to_string())])
            } else {
                t("h_temp_na_m", &[])
            },
            Value::Null,
        );
        v["temperature"] = json!(temp);
        items.push(v);
    } else {
        items.extend(temp_items);
    }
    let failed = list_units(Some("failed"))
        .into_iter()
        .filter(|u| u["unit"].as_str().is_some_and(|s| s.ends_with(".service")))
        .collect::<Vec<_>>();
    if failed.is_empty() {
        items.push(item(
            "services",
            "ok",
            t("h_svc_ok_t", &[]),
            t("h_svc_ok_m", &[]),
            Value::Null,
        ));
    } else {
        let mut names = failed
            .iter()
            .take(4)
            .map(|u| {
                u["description"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| u["unit"].as_str())
                    .unwrap_or("")
            })
            .collect::<Vec<_>>()
            .join(", ");
        if failed.len() > 4 {
            names.push_str(" …")
        }
        let mut v = item(
            "services",
            "warning",
            t(
                if failed.len() == 1 {
                    "h_svc_warn_t"
                } else {
                    "h_svc_warn_t_pl"
                },
                &[("n", failed.len().to_string())],
            ),
            t("h_svc_warn_m", &[("names", names)]),
            action(&lang, "a_services", "services"),
        );
        v["units"] = json!(failed.iter().map(|u| u["unit"].clone()).collect::<Vec<_>>());
        items.push(v);
    }
    if let Ok(upd) = ctx.call("system.updates", &json!({})) {
        if truth(&upd) && truth(&upd["tool"]) {
            let n = &upd["available"];
            if truth(n) {
                let mut message = if n == 1 {
                    t("h_upd_info_m1", &[])
                } else {
                    t(
                        "h_upd_info_m",
                        &[("n", n.as_i64().unwrap_or(0).to_string())],
                    )
                };
                if truth(&upd["security"]) {
                    message.push_str(&t(
                        "h_upd_sec_m",
                        &[("s", upd["security"].as_i64().unwrap_or(0).to_string())],
                    ));
                }
                items.push(item("updates","info",t("h_upd_info_t",&[]),message,json!({"label":t("a_updates",&[]),"app":"settings","params":{"page":"update"},"method":"system.openUpdater"})));
            } else {
                items.push(item(
                    "updates",
                    "ok",
                    t("h_upd_ok_t", &[]),
                    t(
                        if n == 0 {
                            "h_upd_ok_m"
                        } else {
                            "h_upd_unknown_m"
                        },
                        &[],
                    ),
                    Value::Null,
                ));
            }
        }
    }
    let trash = ctx
        .call("storage.trashSize", &json!({}))
        .or_else(|_| ctx.call("files.trashInfo", &json!({})));
    if let Ok(trash) = trash {
        if let Some(size) = trash["size"].as_f64() {
            let mut v = if size >= 1024f64.powi(3) {
                item(
                    "trash",
                    "info",
                    t("h_trash_info_t", &[]),
                    t("h_trash_info_m", &[("size", bytes(size))]),
                    json!({"label":t("a_free",&[]),"app":"taskmgr","params":{"page":"storage","section":"cleanup","tab":"cleanup"}}),
                )
            } else {
                item(
                    "trash",
                    "ok",
                    t("h_trash_ok_t", &[]),
                    t("h_trash_ok_m", &[]),
                    Value::Null,
                )
            };
            v["size"] = trash["size"].clone();
            items.push(v);
        }
    }
    items.sort_by_key(|v| match v["severity"].as_str().unwrap_or("") {
        "critical" => 0,
        "warning" => 1,
        "info" => 2,
        "ok" => 3,
        _ => 9,
    });
    json!(items)
}
