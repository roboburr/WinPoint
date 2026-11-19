//! Fingerprint sign-in through fprintd (`net.reactivated.Fprint` on the system bus), as GNOME Settings' fingerprint
//! dialog does: the default reader, Claim for the own user, EnrollStart with its EnrollStatus signals, ListEnrolledFingers,
//! DeleteEnrolledFingers2, Release. fprintd checks polkit itself (`net.reactivated.fprint.device.enroll`), so the
//! claim, the enrolment and the delete go through [`call_interactive`].
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::{self, call_interactive};
use glib::variant::ToVariant;
use serde_json::{Value, json};

pub(crate) const NAME: &str = "net.reactivated.Fprint";
pub(crate) const MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
pub(crate) const MANAGER_IFACE: &str = "net.reactivated.Fprint.Manager";
pub(crate) const DEVICE_IFACE: &str = "net.reactivated.Fprint.Device";
const TIMEOUT_MS: i32 = 5000;

/// fprintd's finger names (fprintd's device.c, the ten that EnrollStart takes).
pub(crate) const FINGERS: &[&str] = &[
    "right-index-finger",
    "left-index-finger",
    "right-thumb",
    "right-middle-finger",
    "right-ring-finger",
    "right-little-finger",
    "left-thumb",
    "left-middle-finger",
    "left-ring-finger",
    "left-little-finger",
];

/// fprintd on a bus: the connection and the name to call (the well-known name, or a test fake's).
#[derive(Clone)]
pub(crate) struct Fprint {
    pub bus: gio::DBusConnection,
    pub name: String,
}

impl Fprint {
    pub(crate) fn system() -> Option<Fprint> {
        Some(Fprint {
            bus: common::system().ok()?,
            name: NAME.into(),
        })
    }

    fn plain(
        &self,
        path: &str,
        method: &str,
        args: Option<&glib::Variant>,
    ) -> Result<glib::Variant, String> {
        common::call(
            &self.bus,
            &self.name,
            path,
            DEVICE_IFACE,
            method,
            args,
            TIMEOUT_MS,
        )
    }

    /// The default reader's object path; None when there is none (Error.NoSuchDevice) or fprintd is not there.
    pub(crate) fn device(&self) -> Option<String> {
        let v = common::call(
            &self.bus,
            &self.name,
            MANAGER_PATH,
            MANAGER_IFACE,
            "GetDefaultDevice",
            None,
            TIMEOUT_MS,
        )
        .ok()?;
        v.child_value(0).str().map(str::to_owned)
    }

    /// The reader and the own enrolled fingers.
    pub(crate) fn get(&self) -> Value {
        let Some(device) = self.device() else {
            return json!({"available": false, "device": Value::Null, "enrolled": []});
        };
        let props =
            common::getall(&self.bus, &self.name, &device, DEVICE_IFACE).unwrap_or(Value::Null);
        // NoEnrolledPrints (and a refusal of the verify action) read as none
        let enrolled: Vec<String> = self
            .plain(&device, "ListEnrolledFingers", Some(&("",).to_variant()))
            .ok()
            .map(|v| {
                let list = v.child_value(0);
                (0..list.n_children())
                    .filter_map(|i| list.child_value(i).str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "available": true,
            "device": {
                "path": device,
                "name": props["name"].as_str().unwrap_or(""),
                "scanType": if props["scan-type"] == "swipe" { "swipe" } else { "press" },
                "stages": props["num-enroll-stages"].as_i64().filter(|n| *n > 0).unwrap_or(5),
            },
            "enrolled": enrolled,
        })
    }

    /// Claim the reader for the own user ("" is the caller), asking polkit. Another program holding it is fprintd's
    /// Error.AlreadyInUse, told by its D-Bus name: fprintd translates its messages into the system's language.
    pub(crate) fn claim(&self, device: &str, lang: Lang) -> Result<(), ApiError> {
        self.bus
            .call_sync(
                Some(&self.name),
                device,
                DEVICE_IFACE,
                "Claim",
                Some(&("",).to_variant()),
                None,
                gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                common::INTERACTIVE_TIMEOUT_MS,
                gio::Cancellable::NONE,
            )
            .map(|_| ())
            .map_err(|e| claim_error(&e.to_string(), lang))
    }

    /// EnrollStart(finger) on a claimed reader.
    pub(crate) fn enroll_start(
        &self,
        device: &str,
        finger: &str,
        lang: Lang,
    ) -> Result<(), ApiError> {
        call_interactive(
            &self.bus,
            &self.name,
            device,
            DEVICE_IFACE,
            "EnrollStart",
            Some(&(finger,).to_variant()),
            lang,
        )
        .map(|_| ())
    }

    /// EnrollStop, ignoring "not enrolling".
    pub(crate) fn enroll_stop(&self, device: &str) {
        let _ = self.plain(device, "EnrollStop", None);
    }

    /// Release the claim, ignoring "not claimed".
    pub(crate) fn release(&self, device: &str) {
        let _ = self.plain(device, "Release", None);
    }

    /// DeleteEnrolledFingers2 on a claimed reader.
    pub(crate) fn delete_all(&self, device: &str, lang: Lang) -> Result<(), ApiError> {
        call_interactive(
            &self.bus,
            &self.name,
            device,
            DEVICE_IFACE,
            "DeleteEnrolledFingers2",
            None,
            lang,
        )
        .map(|_| ())
    }
}

/// fprintd's error for a reader another program holds.
pub(crate) const ALREADY_IN_USE: &str = "net.reactivated.Fprint.Error.AlreadyInUse";

/// The ApiError of a failed Claim: users.fingerprint.busy for Error.AlreadyInUse (by name, whatever language the
/// message is in), else call_interactive's mapping.
pub(crate) fn claim_error(text: &str, lang: Lang) -> ApiError {
    if common::remote_error_name(text) == Some(ALREADY_IN_USE) {
        ApiError::tr("failed", lang, "users.fingerprint.busy", NO_PARAMS)
    } else {
        common::interactive_error(text, lang)
    }
}

/// An EnrollStatus result: whether it counts as one more stage done (fprintd's "enroll-stage-passed"), and whether
/// the enrolment ended well ("enroll-completed").
pub(crate) fn stage_passed(result: &str) -> bool {
    result == "enroll-stage-passed" || result == "enroll-completed"
}

/// The enrolment's state while it runs (the reader is claimed): the stages so far.
#[derive(Clone, Debug, Default)]
pub(crate) struct Enrolment {
    pub device: String,
    pub stages: i64,
    pub passed: i64,
}

impl Enrolment {
    /// One EnrollStatus signal: the progress event's data.
    pub(crate) fn status(&mut self, result: &str, done: bool) -> Value {
        if result == "enroll-completed" {
            self.passed = self.stages.max(1);
        } else if stage_passed(result) {
            self.passed = (self.passed + 1).min(self.stages.max(1));
        }
        json!({"stage": self.passed, "stages": self.stages, "done": done, "result": result})
    }
}
