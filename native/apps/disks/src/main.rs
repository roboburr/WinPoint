//! Win11 GNOME Disks in GTK 4 (see ../../APPS.md): the Schijven app of vendor/ui/js/apps/disks on the shared
//! kit. Windows 11 Instellingen > Systeem > Opslag > "Schijven en volumes" merged with Schijfbeheer: a card per
//! disk with a proportional partition bar and its volumes; properties pages for a volume and a disk; the
//! format, label, delete, new volume, unlock and initialise dialogs; live through disks.changed.
mod actions;
mod app;
mod art;
mod model;
mod overview;
mod properties;
mod strings;
mod widgets;

use kit::prelude::*;

const USAGE: &str = concat!(
    "win11-gtk-disks [--params JSON]\n",
    "\n",
    "  --params '{\"volume\":\"<id or /dev/...>\"}'   open a volume's properties\n",
    "  --params '{\"disk\":\"<id or /dev/...>\"}'     open a disk's properties\n",
    "  --params '{\"select\":\"<id or /dev/...>\"}'   select a volume on the overview\n",
    "  --params '{\"page\":\"overview|volume|disk\",\"id\":\"...\"}'\n",
    "  \"open\": \"format|label|delete|unlock|new|initialize|menu|disk-menu|free-menu\" also opens that dialog or menu\n",
);

const SPEC: kit::AppSpec = kit::AppSpec::new("disks", "io.github.win11shell.Disks", "Schijven", "Disks").icon("disk").usage(USAGE);

fn main() -> glib::ExitCode {
    kit::run(SPEC, |ctx, args, first| {
        let params = app::parse_args(&args);
        if first {
            kit::theme::install_app_css(include_str!("../css/disks.css"));
            app::App::start(ctx, params);
            return;
        }
        // a second launch reaches the running app: bring it to the front and follow its deep link
        if let Some(app) = app::App::get() {
            app.win.present();
            if params.as_object().is_some_and(|o| !o.is_empty()) {
                app.apply_params(params);
            }
        }
    })
}
