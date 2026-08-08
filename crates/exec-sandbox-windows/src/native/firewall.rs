use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::{S_OK, VARIANT_TRUE};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule3, INetFwRules, NetFwPolicy2, NetFwRule, NET_FW_ACTION_BLOCK,
    NET_FW_IP_PROTOCOL_ANY, NET_FW_MODIFY_STATE, NET_FW_MODIFY_STATE_OK, NET_FW_PROFILE2_ALL,
    NET_FW_RULE_DIR_OUT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};

use std::net::IpAddr;

const NON_LOOPBACK: &str = "0.0.0.0-126.255.255.255,128.0.0.0-255.255.255.255,::,::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff";
const LOOPBACK: &str = "127.0.0.0/8,::/127";

struct RuleSpec {
    name: &'static str,
    description: &'static str,
    protocol: i32,
    remote_addresses: &'static str,
}

const LEGACY_RULES: [&str; 2] = [
    "agent_sandbox_offline_block_loopback_tcp",
    "agent_sandbox_offline_block_loopback_udp",
];

const RULES: [RuleSpec; 2] = [
    RuleSpec {
        name: "agent_sandbox_offline_block_outbound",
        description: "Agent Desktop Sandbox Offline - Block Non-Loopback Outbound",
        protocol: NET_FW_IP_PROTOCOL_ANY.0,
        remote_addresses: NON_LOOPBACK,
    },
    RuleSpec {
        name: "agent_sandbox_offline_block_loopback",
        description: "Agent Desktop Sandbox Offline - Block All Loopback",
        protocol: NET_FW_IP_PROTOCOL_ANY.0,
        remote_addresses: LOOPBACK,
    },
];

pub fn ensure_network_denied(sid: &str) -> Result<(), String> {
    with_rules(|rules| {
        for spec in &RULES {
            let rule = get_or_create_rule(rules, spec, sid)?;
            configure_rule(&rule, spec, sid)?;
            verify_rule(&rule, spec, sid)?;
        }
        for name in LEGACY_RULES {
            let name = BSTR::from(name);
            if unsafe { rules.Item(&name) }.is_ok() {
                unsafe { rules.Remove(&name) }.map_err(|error| {
                    format!("remove legacy Windows sandbox firewall rule: {error}")
                })?;
            }
        }
        Ok(())
    })
}

pub fn verify_network_denied(sid: &str) -> Result<(), String> {
    with_rules(|rules| {
        for spec in &RULES {
            let rule: INetFwRule3 = unsafe { rules.Item(&BSTR::from(spec.name)) }
                .map_err(|error| {
                    format!(
                        "Windows sandbox firewall rule {} is missing: {error}",
                        spec.name
                    )
                })?
                .cast()
                .map_err(|error| {
                    format!("read Windows sandbox firewall rule {}: {error}", spec.name)
                })?;
            verify_rule(&rule, spec, sid)?;
        }
        Ok(())
    })
}

fn with_rules<T>(operation: impl FnOnce(&INetFwRules) -> Result<T, String>) -> Result<T, String> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    initialized
        .ok()
        .map_err(|error| format!("initialize Windows firewall COM: {error}"))?;
    let result = unsafe {
        (|| {
            let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("open Windows firewall policy: {error}"))?;
            ensure_policy_effective(&policy)?;
            let rules = policy
                .Rules()
                .map_err(|error| format!("open Windows firewall rules: {error}"))?;
            operation(&rules)
        })()
    };
    let _ = initialized;
    unsafe { CoUninitialize() };
    result
}

fn ensure_policy_effective(policy: &INetFwPolicy2) -> Result<(), String> {
    let mut state = NET_FW_MODIFY_STATE::default();
    let result = unsafe {
        (Interface::vtable(policy).LocalPolicyModifyState)(Interface::as_raw(policy), &mut state)
    };
    if result != S_OK || state != NET_FW_MODIFY_STATE_OK {
        return Err(format!(
            "local Windows firewall rules are not effective: result={result:?}, state={state:?}"
        ));
    }
    Ok(())
}

fn get_or_create_rule(
    rules: &INetFwRules,
    spec: &RuleSpec,
    sid: &str,
) -> Result<INetFwRule3, String> {
    let name = BSTR::from(spec.name);
    if let Ok(existing) = unsafe { rules.Item(&name) } {
        return existing
            .cast()
            .map_err(|error| format!("open Windows firewall rule {}: {error}", spec.name));
    }
    let rule: INetFwRule3 = unsafe { CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| format!("create Windows firewall rule {}: {error}", spec.name))?;
    unsafe { rule.SetName(&name) }
        .map_err(|error| format!("name Windows firewall rule {}: {error}", spec.name))?;
    configure_rule(&rule, spec, sid)?;
    unsafe { rules.Add(&rule) }
        .map_err(|error| format!("install Windows firewall rule {}: {error}", spec.name))?;
    Ok(rule)
}

fn configure_rule(rule: &INetFwRule3, spec: &RuleSpec, sid: &str) -> Result<(), String> {
    let local_user = BSTR::from(format!("O:LSD:(A;;CC;;;{sid})"));
    unsafe {
        rule.SetDescription(&BSTR::from(spec.description))
            .and_then(|()| rule.SetDirection(NET_FW_RULE_DIR_OUT))
            .and_then(|()| rule.SetAction(NET_FW_ACTION_BLOCK))
            .and_then(|()| rule.SetEnabled(VARIANT_TRUE))
            .and_then(|()| rule.SetProfiles(NET_FW_PROFILE2_ALL.0))
            .and_then(|()| rule.SetProtocol(spec.protocol))
            .and_then(|()| rule.SetRemoteAddresses(&BSTR::from(spec.remote_addresses)))
            .and_then(|()| rule.SetLocalUserAuthorizedList(&local_user))
    }
    .map_err(|error| format!("configure Windows firewall rule {}: {error}", spec.name))
}

fn verify_rule(rule: &INetFwRule3, spec: &RuleSpec, sid: &str) -> Result<(), String> {
    let enabled = unsafe { rule.Enabled() }.map_err(|error| {
        format!(
            "verify Windows firewall rule {} enabled: {error}",
            spec.name
        )
    })?;
    let action = unsafe { rule.Action() }
        .map_err(|error| format!("verify Windows firewall rule {} action: {error}", spec.name))?;
    let direction = unsafe { rule.Direction() }.map_err(|error| {
        format!(
            "verify Windows firewall rule {} direction: {error}",
            spec.name
        )
    })?;
    let protocol = unsafe { rule.Protocol() }.map_err(|error| {
        format!(
            "verify Windows firewall rule {} protocol: {error}",
            spec.name
        )
    })?;
    let profiles = unsafe { rule.Profiles() }.map_err(|error| {
        format!(
            "verify Windows firewall rule {} profiles: {error}",
            spec.name
        )
    })?;
    let remote_addresses = unsafe { rule.RemoteAddresses() }
        .map_err(|error| {
            format!(
                "verify Windows firewall rule {} addresses: {error}",
                spec.name
            )
        })?
        .to_string();
    let users = unsafe { rule.LocalUserAuthorizedList() }
        .map_err(|error| {
            format!(
                "verify Windows firewall rule {} identity: {error}",
                spec.name
            )
        })?
        .to_string();
    if enabled != VARIANT_TRUE
        || action != NET_FW_ACTION_BLOCK
        || direction != NET_FW_RULE_DIR_OUT
        || protocol != spec.protocol
        || profiles != NET_FW_PROFILE2_ALL.0
        || normalized_scope(&remote_addresses) != normalized_scope(spec.remote_addresses)
        || !users.contains(sid)
    {
        return Err(format!(
            "Windows sandbox firewall rule {} failed read-back verification",
            spec.name
        ));
    }
    Ok(())
}

fn normalized_scope(value: &str) -> Vec<String> {
    let mut entries = value
        .split(',')
        .map(canonical_scope_entry)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    entries.sort();
    entries.dedup();
    entries
}

fn canonical_scope_entry(entry: &str) -> String {
    let entry = entry.trim().to_ascii_lowercase();
    if let Some((address, prefix)) = entry.rsplit_once('/') {
        if let Ok(address) = address.parse::<IpAddr>() {
            if let Ok(prefix) = prefix.parse::<u8>() {
                return format!("{address}/{prefix}");
            }
            if let Ok(mask) = prefix.parse::<IpAddr>() {
                if let Some(prefix) = netmask_prefix(address, mask) {
                    return format!("{address}/{prefix}");
                }
            }
        }
    }
    if let Ok(address) = entry.parse::<IpAddr>() {
        return address.to_string();
    }
    if let Some((start, end)) = entry.split_once('-') {
        if let (Ok(start), Ok(end)) = (start.parse::<IpAddr>(), end.parse::<IpAddr>()) {
            if start == end {
                return start.to_string();
            }
            return format!("{start}-{end}");
        }
    }
    entry
}

fn netmask_prefix(address: IpAddr, mask: IpAddr) -> Option<u32> {
    let (mask, width) = match (address, mask) {
        (IpAddr::V4(_), IpAddr::V4(mask)) => (u32::from(mask) as u128, 32),
        (IpAddr::V6(_), IpAddr::V6(mask)) => (u128::from(mask), 128),
        _ => return None,
    };
    let aligned = mask << (128 - width);
    let prefix = aligned.leading_ones();
    let expected = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    (aligned == expected).then_some(prefix)
}

#[cfg(test)]
mod tests {
    use super::normalized_scope;

    #[test]
    fn firewall_scope_normalization_accepts_windows_singleton_ranges() {
        assert_eq!(normalized_scope("::"), normalized_scope("::-::"));
        assert_eq!(
            normalized_scope("0:0:0:0:0:0:0:2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            normalized_scope("::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")
        );
    }

    #[test]
    fn firewall_scope_normalization_is_order_independent() {
        assert_eq!(
            normalized_scope("127.0.0.0/8,::/127"),
            normalized_scope("0:0:0:0:0:0:0:0/127, 127.0.0.0/8")
        );
    }

    #[test]
    fn firewall_scope_normalization_accepts_windows_ipv4_netmasks() {
        assert_eq!(
            normalized_scope("127.0.0.0/8"),
            normalized_scope("127.0.0.0/255.0.0.0")
        );
    }

    #[test]
    fn firewall_scope_normalization_keeps_distinct_ranges_distinct() {
        assert_ne!(normalized_scope("::"), normalized_scope("::-::1"));
    }
}
