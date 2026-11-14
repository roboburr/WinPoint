//! Win11 GNOME Alles (`win11-gtk-alles`): the Everything-style instant file name search in GTK 4, the native
//! port of the WebKit app (vendor/ui/js/apps/alles/, css/alles.css; see ../../APPS.md).
//!
//! One process per session (GApplication "io.github.win11shell.Search", HANDLES_COMMAND_LINE): a second launch
//! hands its arguments to the running one, which brings its window to the front and searches (a deep link).
//!
//!   win11-gtk-alles                                   the search, empty
//!   win11-gtk-alles --params '{"query":"foto"}'       search for "foto" (also q / search / text)
//!   win11-gtk-alles --params '{"path":"/home/robin"}' search in that folder (in:"/home/robin")
//!   win11-gtk-alles --params '{"filter":"picture"}'   one kind (all audio compressed document executable
//!                                                     folder picture video); "pane":"preview|tips|none"
//!   win11-gtk-alles /some/folder                      search in that folder
//!
//! The backend is `win11-app --app alles --http-only` (kit::run starts it), which owns the index worker; the
//! search goes through its index.* methods, the actions through files.* and shell.openApp.
mod actions;
mod art;
mod clamp;
mod highlight;
mod model;
mod options;
mod panes;
mod table;
mod text;
mod ui;
mod wrap;

use kit::prelude::*;

const SPEC: kit::AppSpec = kit::AppSpec::new("alles", "io.github.win11shell.Search", "Alles", "Alles").icon("search").usage(
    "win11-gtk-alles [--params JSON | FOLDER]\n\
     \n\
     \x20 --params '{\"query\":\"foto\"}'        search for this (also \"q\", \"search\", \"text\")\n\
     \x20 --params '{\"path\":\"/home/robin\"}'  search in this folder\n\
     \x20 --params '{\"filter\":\"picture\"}'    all, audio, compressed, document, executable, folder, picture, video\n\
     \x20 --params '{\"pane\":\"preview\"}'      preview, tips or none\n\
     \x20 FOLDER                              search in this folder\n\
     \n\
     A second launch hands its arguments to the running Alles.\n",
);

fn main() -> glib::ExitCode {
    kit::run(SPEC, |ctx, args, first| {
        let params = ui::Params::parse(&args);
        if first {
            ui::App::start(ctx, params);
        } else {
            ui::App::navigate(params);
        }
    })
}
