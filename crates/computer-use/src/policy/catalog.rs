pub(super) const FORBIDDEN_BUNDLE_IDS: &[(&str, &str)] = &[
    (
        "com.apple.Terminal",
        "terminal and shell applications are never controllable",
    ),
    (
        "com.googlecode.iterm2",
        "terminal and shell applications are never controllable",
    ),
    (
        "dev.warp.Warp-Stable",
        "terminal and shell applications are never controllable",
    ),
    (
        "com.github.wez.wezterm",
        "terminal and shell applications are never controllable",
    ),
    (
        "org.alacritty",
        "terminal and shell applications are never controllable",
    ),
    (
        "io.alacritty",
        "terminal and shell applications are never controllable",
    ),
    (
        "net.kovidgoyal.kitty",
        "terminal and shell applications are never controllable",
    ),
    (
        "com.mitchellh.ghostty",
        "terminal and shell applications are never controllable",
    ),
    (
        "co.zeit.hyper",
        "terminal and shell applications are never controllable",
    ),
    (
        "com.termius-dmg.mac",
        "terminal and shell applications are never controllable",
    ),
    (
        "com.vandyke.SecureCRT",
        "terminal and shell applications are never controllable",
    ),
    (
        "org.tabby",
        "terminal and shell applications are never controllable",
    ),
    (
        "com.apple.keychainaccess",
        "password and keychain applications are never controllable",
    ),
    (
        "com.apple.Passwords",
        "password and keychain applications are never controllable",
    ),
    (
        "com.1password.1password",
        "password and keychain applications are never controllable",
    ),
    (
        "com.bitwarden.desktop",
        "password and keychain applications are never controllable",
    ),
    (
        "com.dashlane.dashlanephonefinal",
        "password and keychain applications are never controllable",
    ),
    (
        "com.lastpass.LastPass",
        "password and keychain applications are never controllable",
    ),
    (
        "org.keepassxc.keepassxc",
        "password and keychain applications are never controllable",
    ),
    (
        "ch.protonmail.protonpass",
        "password and keychain applications are never controllable",
    ),
    (
        "com.callpod.keepermac",
        "password and keychain applications are never controllable",
    ),
    (
        "com.nordsec.nordpass",
        "password and keychain applications are never controllable",
    ),
    (
        "in.sinew.Enpass-Desktop",
        "password and keychain applications are never controllable",
    ),
    (
        "com.apple.systempreferences",
        "macOS privacy and security settings are never controllable",
    ),
    (
        "com.apple.SecurityAgent",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "com.apple.securityagent",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "com.apple.AuthorizationUI",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "com.apple.LocalAuthentication.UIAgent",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "com.apple.loginwindow",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "com.apple.CoreServicesUIAgent",
        "authentication and administrator dialogs are never controllable",
    ),
];

pub(super) const FORBIDDEN_BUNDLE_FRAGMENTS: &[(&str, &str)] = &[
    (
        "securityagent",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "authorizationui",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "localauthentication",
        "authentication and administrator dialogs are never controllable",
    ),
    (
        "passwordmanager",
        "password and keychain applications are never controllable",
    ),
    (
        "password-manager",
        "password and keychain applications are never controllable",
    ),
    (
        "keychain",
        "password and keychain applications are never controllable",
    ),
    (
        "1password",
        "password and keychain applications are never controllable",
    ),
    (
        "bitwarden",
        "password and keychain applications are never controllable",
    ),
    (
        "dashlane",
        "password and keychain applications are never controllable",
    ),
    (
        "lastpass",
        "password and keychain applications are never controllable",
    ),
    (
        "keepass",
        "password and keychain applications are never controllable",
    ),
    (
        "protonpass",
        "password and keychain applications are never controllable",
    ),
    (
        "nordpass",
        "password and keychain applications are never controllable",
    ),
    (
        "keeper",
        "password and keychain applications are never controllable",
    ),
    (
        "enpass",
        "password and keychain applications are never controllable",
    ),
    (
        "termius",
        "terminal and shell applications are never controllable",
    ),
    (
        "securecrt",
        "terminal and shell applications are never controllable",
    ),
    (
        "wezterm",
        "terminal and shell applications are never controllable",
    ),
    (
        "alacritty",
        "terminal and shell applications are never controllable",
    ),
    (
        "ghostty",
        "terminal and shell applications are never controllable",
    ),
];

pub(super) const FORBIDDEN_APP_NAMES: &[&str] = &[
    "terminal",
    "iterm",
    "warp",
    "wezterm",
    "alacritty",
    "kitty",
    "ghostty",
    "hyper",
    "termius",
    "securecrt",
    "tabby",
    "rio",
    "contour",
    "xterm",
    "keychain access",
    "passwords",
    "1password",
    "bitwarden",
    "dashlane",
    "lastpass",
    "keepassxc",
    "proton pass",
    "keeper",
    "nordpass",
    "enpass",
    "system settings",
    "system preferences",
    "securityagent",
    "authorizationui",
];

pub(super) const BROWSER_BUNDLE_IDS: &[&str] = &[
    "com.apple.safari",
    "com.google.chrome",
    "org.chromium.chromium",
    "company.thebrowser.browser",
    "com.brave.browser",
    "com.microsoft.edgemac",
    "org.mozilla.firefox",
    "org.mozilla.firefoxdeveloperedition",
    "org.mozilla.nightly",
    "com.operasoftware.opera",
    "com.operasoftware.operagx",
    "com.vivaldi.vivaldi",
    "com.duckduckgo.macos.browser",
    "com.kagi.kagimacos",
    "com.kagi.orion",
];

pub(super) const BROWSER_APP_NAMES: &[&str] = &[
    "safari",
    "google chrome",
    "chromium",
    "arc",
    "brave browser",
    "microsoft edge",
    "firefox",
    "opera",
    "vivaldi",
    "duckduckgo",
    "orion",
    "dia",
];
