use super::*;

#[test]
fn friendly_runtime_formats_known_shape() {
    assert_eq!(
        friendly_runtime("com.apple.CoreSimulator.SimRuntime.iOS-26-5"),
        "iOS 26.5"
    );
    assert_eq!(
        friendly_runtime("unrecognized-format"),
        "unrecognized-format"
    );
}

#[test]
fn parses_real_simctl_list_json_shape() {
    // Captured live from `xcrun simctl list devices --json` (trimmed).
    let json = r#"{
            "devices" : {
              "com.apple.CoreSimulator.SimRuntime.iOS-26-5" : [
                {
                  "dataPath" : "/tmp",
                  "dataPathSize" : 1,
                  "logPath" : "/tmp",
                  "udid" : "83EAE099-5C74-465D-9FAE-CC86D32D7A20",
                  "isAvailable" : true,
                  "deviceTypeIdentifier" : "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro",
                  "state" : "Shutdown",
                  "name" : "iPhone 17 Pro"
                }
              ]
            }
        }"#;
    let parsed: SimctlList = serde_json::from_str(json).unwrap();
    let devices = parsed
        .devices
        .get("com.apple.CoreSimulator.SimRuntime.iOS-26-5")
        .unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].udid, "83EAE099-5C74-465D-9FAE-CC86D32D7A20");
    assert_eq!(devices[0].name, "iPhone 17 Pro");
    assert_eq!(devices[0].state, "Shutdown");
    assert!(devices[0].is_available);
}

#[test]
fn already_in_state_matches_real_simctl_error_text() {
    // Captured live from `xcrun simctl boot`/`shutdown` on an
    // already-booted/shutdown device.
    assert!(already_in_state(
            "An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=405):\nUnable to boot device in current state: Booted\n"
        ));
    assert!(already_in_state(
            "An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=405):\nUnable to shutdown device in current state: Shutdown\n"
        ));
    assert!(!already_in_state("some other simctl error"));
}

#[test]
fn idb_button_maps_known_buttons() {
    assert_eq!(idb_button("home").unwrap(), "HOME");
    assert!(idb_button("bogus").is_err());
}
