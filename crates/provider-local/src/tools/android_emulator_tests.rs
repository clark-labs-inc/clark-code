use super::*;

#[test]
fn parses_adb_devices_output() {
    let stdout = "List of devices attached\nemulator-5554\tdevice product:sdk_gphone64_arm64\n\n";
    let devices = parse_adb_devices(stdout);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].serial, "emulator-5554");
    assert_eq!(devices[0].state, "device");
}

#[test]
fn escapes_spaces_and_shell_metacharacters() {
    assert_eq!(escape_adb_text("hello world"), "hello%sworld");
    assert_eq!(escape_adb_text("a&b"), "a\\&b");
    assert_eq!(escape_adb_text("it's"), "it\\'s");
}

#[test]
fn keyevent_code_maps_known_buttons() {
    assert_eq!(keyevent_code("home").unwrap(), "3");
    assert_eq!(keyevent_code("back").unwrap(), "4");
    assert!(keyevent_code("bogus").is_err());
}
