//! "Eigenschappen" of a volume and of a disk (vendor/ui/js/apps/disks/properties.js): label, mount point,
//! encryption, format, delete and the details; drive health (SMART / NVMe health log with all values), safe
//! removal, eject, new volume, initialise, the volumes on the disk and the disk's details.
use crate::actions;
use crate::app::App;
use crate::art;
use crate::model::{self, arr, bytes, num, opt_num, st, truthy};
use crate::overview;
use crate::strings::{s, tf, tp};
use crate::widgets::{Detail, details_grid, find_label, hbox, label, usage_bar, vbox};
use kit::controls::{self, Expander, InfoBar, ProgressRing, SettingsCard, SettingsGroup, Severity, Toast, Variant};
use kit::format;
use kit::prelude::*;
use std::rc::Rc;

fn reason_text(reasons: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    for r in arr(reasons) {
        let r = st(r);
        if r == "boot_entry" {
            continue;
        }
        let t = s(&format!("r_{r}"));
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out.join(" ")
}

fn boot_names(names: &Value) -> String {
    arr(names).iter().map(|n| format!("\u{2018}{}\u{2019}", st(n))).collect::<Vec<_>>().join(", ")
}

fn bar(sev: Severity, title: &str, msg: &str) -> gtk::Widget {
    let b = InfoBar::new(sev, title, msg);
    b.set_closable(false);
    b.widget().clone().upcast()
}

fn button(text: &str, variant: Variant, enabled: bool, f: impl Fn() + 'static) -> gtk::Button {
    let b = controls::button(text, variant);
    b.set_sensitive(enabled);
    b.connect_clicked(move |_| f());
    b
}

/// A settings card with a button as its control.
fn card(icon: &str, title: &str, desc: &str, control: &gtk::Button, name: &str) -> gtk::Widget {
    let c = SettingsCard::new(icon, title, desc);
    c.set_control(control);
    control.set_widget_name(name);
    c.widget().clone()
}

fn group(title: &str, items: Vec<gtk::Widget>) -> gtk::Widget {
    let g = SettingsGroup::new(title, "");
    for w in items {
        g.add(&w);
    }
    g.widget().clone().upcast()
}

/// The details card (`.fl-settings-card.dk-details-card`).
fn details_card(rows: Vec<Option<(String, Detail)>>) -> gtk::Widget {
    let c = vbox(0, "dk-card dk-details-card");
    c.append(&details_grid(rows));
    c.upcast()
}

fn hero(art_kind: &str, system: bool, title: &str, sub: &str, state: Option<&str>, end: Option<gtk::Widget>) -> (gtk::Box, gtk::Box) {
    let h = vbox(16, "dk-hero");
    let top = hbox(18, "dk-hero-top");
    let ab = hbox(0, "dk-hero-art");
    ab.set_size_request(60, 60);
    ab.set_valign(gtk::Align::Center);
    let a = art::drive_art(art_kind, 56, system);
    a.set_hexpand(true);
    ab.append(&a);
    ab.set_hexpand(false);
    top.append(&ab);
    let text = vbox(2, "dk-hero-text");
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let tl = label(title, "dk-hero-title", false);
    tl.set_tooltip_text(Some(title));
    text.append(&tl);
    text.append(&label(sub, "dk-hero-sub", true));
    if let Some(st) = state {
        text.append(&label(st, "dk-hero-state", true));
    }
    top.append(&text);
    if let Some(e) = end {
        e.set_valign(gtk::Align::Center);
        top.append(&e);
    }
    h.append(&top);
    (h, top)
}

// ------------------------------------------------------------------------------------------------ volume

pub fn volume_page(app: &Rc<App>, d: &Value, v: &Value) -> Vec<gtk::Widget> {
    let mut out: Vec<gtk::Widget> = Vec::new();
    let enc = &v["encrypted"];
    let locked = truthy(enc) && !truthy(&enc["unlocked"]);
    let busy = truthy(&v["busy"]);

    // header card: art, name, usage
    let sub = [
        st(&d["name"]),
        if truthy(&v["partNumber"]) { tf("partition", &[("n", &st(&v["partNumber"]))]) } else { String::new() },
        st(&v["fsName"]),
        bytes(&v["size"]),
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join(" \u{b7} ");
    let open = if (truthy(&v["mounted"]) || truthy(&v["canMount"])) && !locked {
        let b = controls::button_with_glyph("folder-open", &s("open"), Variant::Standard);
        b.set_widget_name("dk-hero-open");
        let (w, vv) = (Rc::downgrade(app), v.clone());
        b.connect_clicked(move |_| {
            if let Some(app) = w.upgrade() {
                actions::open_in_explorer(&app, &vv);
            }
        });
        Some(b.upcast::<gtk::Widget>())
    } else {
        None
    };
    let status = model::volume_status(v);
    let (h, _) = hero(&st(&d["kind"]), truthy(&v["isSystem"]), &model::title_with_mount(v), &sub, Some(&status), open);
    if truthy(&v["total"]) {
        let u = vbox(8, "dk-hero-usage");
        u.append(&usage_bar(num(&v["used"]), num(&v["total"]), true));
        let nums = hbox(0, "dk-hero-nums");
        let part = |amount: &Value, word: &str| {
            let b = hbox(4, "");
            let a = label(&bytes(amount), "dk-hero-num", false);
            a.set_valign(gtk::Align::Baseline);
            let w = label(&s(word).to_lowercase(), "", false);
            w.set_valign(gtk::Align::Baseline);
            b.append(&a);
            b.append(&w);
            b
        };
        let used = part(&v["used"], "used");
        used.set_hexpand(true);
        nums.append(&used);
        nums.append(&part(&v["free"], "free"));
        u.append(&nums);
        h.append(&u);
    }
    out.push(h.upcast());

    // why some buttons are off
    if truthy(&v["isSystem"]) {
        out.push(bar(Severity::Info, &s("protectedTitle"), &format!("{} {}", reason_text(&v["protected"]), s("protectedBody"))));
    } else if !arr(&v["bootEntries"]).is_empty() {
        out.push(bar(Severity::Info, &s("bootTitle"), &tf("bootBody", &[("names", &boot_names(&v["bootEntries"]))])));
    } else if v["role"] == "system" {
        out.push(bar(Severity::Warning, &s("systemPartTitle"), &s("systemPartBody")));
    } else if truthy(&d["dynamic"]) {
        out.push(bar(Severity::Warning, &s("dynamicTitle"), &s("dynamicBody")));
    }

    // actions
    let mut cards = Vec::new();
    let fs = st(&v["fsType"]);
    let w = Rc::downgrade(app);
    if !fs.is_empty() && !truthy(&v["isSwap"]) && fs != "crypto_LUKS" {
        let (w, vv) = (w.clone(), v.clone());
        let b = button(&s("changeLabel"), Variant::Standard, truthy(&v["canLabel"]) && !busy, move || {
            if let Some(app) = w.upgrade() {
                actions::change_label(&app, &vv);
            }
        });
        let lbl = st(&v["label"]);
        let desc = if lbl.is_empty() { s("noLabel") } else { lbl };
        cards.push(card("rename", &s("label"), &desc, &b, "dk-label"));
    }
    if truthy(enc) {
        let kind = { let k = st(&enc["type"]); if k.is_empty() { "LUKS".to_owned() } else { k.replacen("crypto_", "", 1) } };
        let b = if locked {
            let (w, vv) = (w.clone(), v.clone());
            button(&format!("{}\u{2026}", s("unlock")), Variant::Accent, true, move || {
                if let Some(app) = w.upgrade() {
                    actions::unlock(&app, &vv);
                }
            })
        } else {
            let (w, vv) = (w.clone(), v.clone());
            button(&s("lock"), Variant::Standard, !truthy(&v["isSystem"]) && !busy, move || {
                if let Some(app) = w.upgrade() {
                    actions::lock(&app, &vv);
                }
            })
        };
        cards.push(card(
            if locked { "lock" } else { "lock-open" },
            &tf("encryption", &[("type", &kind)]),
            &if locked { s("encLocked") } else { s("encUnlocked") },
            &b,
            if locked { "dk-unlock" } else { "dk-lock" },
        ));
    }
    if !locked && !fs.is_empty() && !truthy(&v["isSwap"]) {
        let mounted = truthy(&v["mounted"]);
        let b = if mounted {
            let (w, vv) = (w.clone(), v.clone());
            button(&s("unmount"), Variant::Standard, truthy(&v["canUnmount"]) && !busy, move || {
                if let Some(app) = w.upgrade() {
                    actions::unmount(&app, &vv);
                }
            })
        } else {
            let (w, vv) = (w.clone(), v.clone());
            button(&s("mount"), Variant::Standard, truthy(&v["canMount"]) && !busy, move || {
                if let Some(app) = w.upgrade() {
                    actions::mount(&app, &vv);
                }
            })
        };
        let mps = arr(&v["mountPoints"]).iter().map(st).collect::<Vec<_>>().join(", ");
        let c = card("folder", &s("mountPoint"), &if mounted { mps } else { s("mountPointDescOff") }, &b, if mounted { "dk-unmount" } else { "dk-mount" });
        if mounted && let Some(l) = find_label(&c, "w11-sc-desc") {
            l.add_css_class("dk-mono");
        }
        cards.push(c);
    }
    {
        let (w, dd, vv) = (w.clone(), d.clone(), v.clone());
        let b = button(&format!("{}\u{2026}", s("format")), Variant::Standard, truthy(&v["canFormat"]) && !busy, move || {
            if let Some(app) = w.upgrade() {
                actions::format(&app, &dd, &vv);
            }
        });
        cards.push(card("storage", &s("format"), &s("formatDesc"), &b, "dk-format"));
    }
    if v["kind"] == "partition" {
        let (w, dd, vv) = (w.clone(), d.clone(), v.clone());
        let b = button(&format!("{}\u{2026}", s("deleteVolume")), Variant::Standard, truthy(&v["canDelete"]) && !busy, move || {
            if let Some(app) = w.upgrade() {
                actions::delete_volume(&app, &dd, &vv);
            }
        });
        cards.push(card("delete", &s("deleteVolume"), &s("deleteDesc"), &b, "dk-delete"));
    }
    out.push(group(&s("volume"), cards));

    // details
    let bytes_exact = |n: &Value| format!("{} ({})", bytes(n), tp("bytes", num(n), &[("n", &format::number(num(n), 0))]));
    let fs_name = st(&v["fsName"]);
    let fs_text = if fs_name.is_empty() {
        "\u{2013}".to_owned()
    } else if truthy(&v["fsVersion"]) {
        format!("{fs_name} ({})", st(&v["fsVersion"]))
    } else {
        fs_name
    };
    let mounts = arr(&v["mountPoints"]).iter().map(st).collect::<Vec<_>>();
    let device = if truthy(&enc["cleartextDevice"]) {
        format!("{} \u{2192} {}", st(&v["device"]), st(&enc["cleartextDevice"]))
    } else {
        st(&v["device"])
    };
    let link = gtk::Button::with_label(&format!("{} \u{b7} {}", st(&d["name"]), st(&d["model"])));
    link.add_css_class("dk-linkbtn");
    link.set_halign(gtk::Align::Start);
    if let Some(l) = link.child().and_downcast::<gtk::Label>() {
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    }
    link.set_widget_name("dk-disk-link");
    let (w2, did) = (w.clone(), st(&d["id"]));
    link.connect_clicked(move |_| {
        if let Some(app) = w2.upgrade() {
            app.open_disk(&did);
        }
    });
    let ptype = st(&v["partTypeName"]);
    let rows = vec![
        Some((s("d_fs"), Detail::Text(fs_text))),
        Some((s("d_size"), Detail::Text(bytes_exact(&v["size"])))),
        truthy(&v["total"]).then(|| {
            let pct = num(&v["used"]) / num(&v["total"]) * 100.0;
            (s("capacity"), Detail::Text(format!("{} \u{b7} {} {} ({})", bytes(&v["total"]), bytes(&v["used"]), s("used").to_lowercase(), format::percent(pct, 0))))
        }),
        Some((s("d_type"), Detail::Text(if !ptype.is_empty() { ptype } else if v["kind"] == "whole" { model::kind_label(d) } else { String::new() }))),
        truthy(&v["partName"]).then(|| (s("d_partName"), Detail::Text(st(&v["partName"])))),
        truthy(&v["partNumber"]).then(|| (s("d_partition"), Detail::Text(st(&v["partNumber"])))),
        (v["kind"] == "partition").then(|| (s("d_offset"), Detail::Text(bytes_exact(&v["offset"])))),
        Some((s("d_device"), Detail::Mono(device))),
        truthy(&v["uuid"]).then(|| (s("d_uuid"), Detail::Mono(st(&v["uuid"])))),
        (!mounts.is_empty()).then(|| (s("d_mounts"), Detail::Mono(mounts.join(", ")))),
        Some((s("d_disk"), Detail::Widget(link.upcast()))),
    ];
    out.push(group(&s("details"), vec![details_card(rows)]));
    out
}

// ------------------------------------------------------------------------------------------------ disk

fn health_card(app: &Rc<App>, d: &Value) -> gtk::Widget {
    let h = model::health_of(d);
    let smart = d["smart"].clone();
    let supported = truthy(&smart["supported"]);
    let sev = match h.as_str() {
        "ok" => "success",
        "warning" => "warning",
        "critical" => "error",
        _ => "info",
    };
    let card = vbox(14, &format!("dk-card dk-healthcard is-{h}"));
    let head = hbox(14, "dk-healthcard-head");
    let icon = kit::icons::severity_icon(sev, 24);
    icon.set_valign(gtk::Align::Center);
    head.append(&icon);
    let text = vbox(0, "dk-healthcard-text");
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&label(&s(&format!("h_{h}")), "dk-healthcard-title", true));
    text.append(&label(&s(&format!("h_{h}_long")), "dk-healthcard-desc", true));
    head.append(&text);
    let ctl = hbox(8, "dk-ctl-row");
    ctl.set_valign(gtk::Align::Center);
    // the busy ring exists only while a check runs (a spinning ring ticks every frame, also when hidden)
    let ring = hbox(0, "dk-ring-slot");
    ring.set_valign(gtk::Align::Center);
    ctl.append(&ring);
    let check = controls::button_with_glyph("refresh", &s("healthCheck"), Variant::Standard);
    check.set_widget_name("dk-health-check");
    check.set_sensitive(supported);
    ctl.append(&check);
    head.append(&ctl);
    card.append(&head);
    let body = vbox(0, "dk-healthcard-grid");
    card.append(&body);
    let advanced = vbox(0, "dk-healthcard-adv");
    card.append(&advanced);

    let fill = {
        let (body, advanced) = (body.downgrade(), advanced.downgrade());
        Rc::new(move |x: &Value| {
            let (Some(body), Some(advanced)) = (body.upgrade(), advanced.upgrade()) else { return };
            let n = |k: &str| opt_num(&x[k]);
            let rows = vec![
                n("lifeRemaining").map(|v| (s("lifeRemaining"), Detail::Text(format::percent(v, 0)))),
                n("availableSpare").map(|v| (s("spare"), Detail::Text(format::percent(v, 0)))),
                n("temperature").map(|v| (s("temperature"), Detail::Text(format!("{} \u{b0}C", format::number(v, 0))))),
                n("powerOnHours").map(|v| (s("powerOn"), Detail::Text(tp("hours", v, &[("n", &format::number(v, 0))])))),
                n("dataWritten").map(|v| (s("written"), Detail::Text(format::bytes(v)))),
                n("dataRead").map(|v| (s("read"), Detail::Text(format::bytes(v)))),
                n("powerCycles").map(|v| (s("powerCycles"), Detail::Text(format::number(v, 0)))),
                n("unsafeShutdowns").map(|v| (s("unsafeShutdowns"), Detail::Text(format::number(v, 0)))),
                n("badSectors").map(|v| (s("badSectors"), Detail::Text(format::number(v, 0)))),
                n("mediaErrors").map(|v| (s("mediaErrors"), Detail::Text(format::number(v, 0)))),
                truthy(&x["selftest"]).then(|| {
                    let t = st(&x["selftest"]);
                    (s("selftest"), Detail::Text(if t == "success" { s("selftestOk") } else { t }))
                }),
            ];
            controls::clear(&body);
            let any = rows.iter().any(Option::is_some);
            if any {
                let g = details_grid(rows);
                g.set_margin_start(38);
                body.append(&g);
            }
            body.set_visible(any);
            controls::clear(&advanced);
            let attrs = arr(&x["attributes"]);
            advanced.set_visible(!attrs.is_empty());
            if !attrs.is_empty() {
                let e = Expander::new("chart", &s("smartAll"), &s("smartAllDesc"));
                e.header_button().set_widget_name("dk-smart-all");
                let sc = gtk::ScrolledWindow::new();
                sc.set_vscrollbar_policy(gtk::PolicyType::Never);
                sc.set_hscrollbar_policy(gtk::PolicyType::Automatic);
                sc.set_propagate_natural_height(true);
                sc.set_child(Some(&attr_table(attrs)));
                e.set_content(&sc);
                advanced.append(e.widget());
            }
        })
    };
    let load: Rc<dyn Fn(bool)> = {
        let (w, ring, check, fill, did, smart) = (Rc::downgrade(app), ring.downgrade(), check.downgrade(), fill.clone(), d["id"].clone(), smart.clone());
        Rc::new(move |refresh: bool| {
            let Some(app) = w.upgrade() else { return };
            if let Some(r) = ring.upgrade() {
                controls::clear(&r);
                r.append(ProgressRing::new(16, None).widget());
            }
            if let Some(c) = check.upgrade() {
                c.set_sensitive(false);
            }
            let (ring, check, fill, smart, wapp) = (ring.clone(), check.clone(), fill.clone(), smart.clone(), w.clone());
            app.ctx.call("disks.smart", json!({ "disk": did, "refresh": refresh }), move |r| {
                let parent = wapp.upgrade().map(|a| a.parent());
                match r {
                    Ok(x) => {
                        let mut merged = smart.clone();
                        if let (Some(m), Some(o)) = (merged.as_object_mut(), x.as_object()) {
                            for (k, v) in o {
                                m.insert(k.clone(), v.clone());
                            }
                        }
                        fill(&merged);
                        if refresh && let Some(p) = &parent {
                            Toast::new(&s("healthChecked")).severity(Severity::Success).timeout(2500).show(p);
                        }
                    }
                    Err(e) => {
                        if refresh && let Some(p) = &parent {
                            Toast::new(&s("failed")).message(&e.message).severity(Severity::Error).timeout(6000).show(p);
                        }
                    }
                }
                if let Some(r) = ring.upgrade() {
                    controls::clear(&r);
                }
                if let Some(c) = check.upgrade() {
                    c.set_sensitive(supported);
                }
            });
        })
    };
    let l = load.clone();
    check.connect_clicked(move |_| l(true));
    fill(&smart);
    if supported {
        load(false);
    }
    card.upcast()
}

/// The SMART attribute table (ID, name, value, worst, threshold, raw; failing rows in red).
fn attr_table(attrs: &[Value]) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.add_css_class("dk-attrs");
    g.set_hexpand(true);
    let heads = ["ID".to_owned(), s("attr"), s("attrValue"), s("attrWorst"), s("attrThreshold"), s("attrRaw")];
    for (c, h) in heads.iter().enumerate() {
        let l = label(h, "dk-attr-head", false);
        l.set_ellipsize(gtk::pango::EllipsizeMode::None);
        if c >= 2 {
            l.set_xalign(1.0);
        }
        g.attach(&l, c as i32, 0, 1, 1);
    }
    let dash = |v: &Value| v.as_f64().map(|n| format::number(n, 0)).unwrap_or_else(|| "\u{2013}".into());
    for (r, a) in attrs.iter().enumerate() {
        let failing = truthy(&a["failing"]);
        let cells = [st(&a["id"]), st(&a["name"]), dash(&a["value"]), dash(&a["worst"]), dash(&a["threshold"]), format::number(num(&a["pretty"]), 0)];
        for (c, text) in cells.iter().enumerate() {
            let mut class = "dk-attr-cell".to_owned();
            if c == 5 {
                class.push_str(" dk-mono");
            }
            if failing {
                class.push_str(" dk-failing");
            }
            let l = label(text, &class, false);
            l.set_ellipsize(gtk::pango::EllipsizeMode::None);
            if c >= 2 {
                l.set_xalign(1.0);
            }
            if c == 1 {
                l.set_hexpand(true);
            }
            if failing {
                l.set_tooltip_text(Some(&s("failingNow")));
            }
            g.attach(&l, c as i32, r as i32 + 1, 1, 1);
        }
    }
    g
}

pub fn disk_page(app: &Rc<App>, d: &Value) -> Vec<gtk::Widget> {
    let mut out: Vec<gtk::Widget> = Vec::new();
    let meta = [model::kind_label(d), bytes(&d["size"]), model::table_long(d)].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" \u{b7} ");
    let (h, _) = hero(&st(&d["kind"]), truthy(&d["isSystem"]), &format!("{}: {}", st(&d["name"]), st(&d["model"])), &meta, None, None);
    let hb = vbox(0, "dk-hero-bar");
    hb.append(&overview::disk_bar(app, d, true));
    h.append(&hb);
    out.push(h.upcast());
    if truthy(&d["isSystem"]) {
        out.push(bar(Severity::Info, &s("protectedDiskTitle"), &s("r_disk")));
    } else if !arr(&d["bootEntries"]).is_empty() {
        out.push(bar(Severity::Info, &s("bootDiskTitle"), &tf("bootDiskBody", &[("names", &boot_names(&d["bootEntries"]))])));
    } else if truthy(&d["dynamic"]) {
        out.push(bar(Severity::Warning, &s("dynamicTitle"), &s("dynamicBody")));
    }
    let busy = model::disk_busy(d);
    out.push(group(&s("health"), vec![health_card(app, d)]));

    let w = Rc::downgrade(app);
    let mut cards = Vec::new();
    if truthy(&d["removable"]) {
        let (w, dd) = (w.clone(), d.clone());
        let b = button(&s("safeRemove"), Variant::Standard, !busy && truthy(&d["canEject"]), move || {
            if let Some(app) = w.upgrade() {
                actions::safe_remove(&app, &dd);
            }
        });
        if busy {
            b.set_tooltip_text(Some(&s("busyDisk")));
        }
        cards.push(card("usb", &s("safeRemove"), &s("safeRemoveDesc"), &b, "dk-saferemove"));
    }
    if truthy(&d["mediaRemovable"]) || d["kind"] == "optical" {
        let (w, dd) = (w.clone(), d.clone());
        let b = button(&s("eject"), Variant::Standard, !busy && truthy(&d["canEject"]), move || {
            if let Some(app) = w.upgrade() {
                actions::eject(&app, &dd);
            }
        });
        if busy {
            b.set_tooltip_text(Some(&s("busyDisk")));
        }
        cards.push(card("arrow-up", &s("eject"), &s("ejectDesc"), &b, "dk-eject"));
    }
    if truthy(&d["partitionTable"]) {
        for (i, f) in arr(&d["free"]).iter().enumerate() {
            let (w, dd, ff) = (w.clone(), d.clone(), f.clone());
            let b = button(&s("newVolumeDots"), Variant::Standard, actions::can_create_in(d, f), move || {
                if let Some(app) = w.upgrade() {
                    actions::new_volume(&app, &dd, &ff);
                }
            });
            let desc = tf(if truthy(&f["logical"]) { "freeSpaceDesc" } else { "newVolumeDesc" }, &[("s", &bytes(&f["size"]))]);
            cards.push(card("plus", &s("newVolume"), &desc, &b, &format!("dk-new-{i}")));
        }
    }
    if d["kind"] != "optical" {
        let used = truthy(&d["partitionTable"]) || !arr(&d["volumes"]).is_empty();
        let (w, dd) = (w.clone(), d.clone());
        let b = button(&format!("{}\u{2026}", s("initialize")), Variant::Standard, actions::can_init(d), move || {
            if let Some(app) = w.upgrade() {
                actions::initialize(&app, &dd);
            }
        });
        cards.push(card(
            "disk",
            &if used { s("reinitialize") } else { s("initialize") },
            &if used { s("reinitializeDesc") } else { s("initializeDesc") },
            &b,
            "dk-initialize",
        ));
    }
    out.push(group(&s("diskSection"), cards));

    let vols: Vec<gtk::Widget> = overview::disk_rows(app, d).into_iter().filter(|(_, free)| !free).map(|(w, _)| w).collect();
    if !vols.is_empty() {
        out.push(group(&s("volumesOnDisk"), vec![overview::rows_box(vols, true).upcast()]));
    }

    let opt = |k: &str, key: &str, mono: bool| {
        truthy(&d[k]).then(|| (s(key), if mono { Detail::Mono(st(&d[k])) } else { Detail::Text(st(&d[k])) }))
    };
    let size = &d["size"];
    let rows = vec![
        Some((s("d_model"), Detail::Text(st(&d["model"])))),
        opt("vendor", "d_vendor", false),
        opt("serial", "d_serial", true),
        opt("revision", "d_firmware", false),
        Some((s("d_kind"), Detail::Text(model::kind_label(d)))),
        Some((s("d_bus"), Detail::Text(model::bus_label(d)))),
        Some((s("capacity"), Detail::Text(format!("{} ({})", bytes(size), tp("bytes", num(size), &[("n", &format::number(num(size), 0))]))))),
        Some((s("d_style"), Detail::Text(model::table_long(d)))),
        Some((s("d_device"), Detail::Mono(st(&d["device"])))),
        opt("wwn", "d_wwn", true),
        opt("loopFile", "d_file", true),
    ];
    out.push(group(&s("details"), vec![details_card(rows)]));
    out
}
