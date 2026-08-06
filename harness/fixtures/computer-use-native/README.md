# Computer-use native fixture

This is a disposable AppKit surface for Clark Code's macOS computer-use smoke
tests. It contains ordinary text, secure-text, button, slider, and status
controls with stable Accessibility labels.

The fixture is debug-only by construction:

- it is built only by `scripts/build-computer-use-native-fixture.sh`;
- it is not listed in any Tauri `externalBin`, resource, or release workflow;
- its Info.plist carries `ClarkDebugOnlyFixture=true`;
- the build script requires an explicit local signing identity and writes to an
  operator-selected temporary directory.

Do not add this app to production packaging. The signed native smoke launches
it from a temporary path, verifies the controlled action, and terminates it.
