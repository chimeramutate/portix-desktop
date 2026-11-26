use crate::profiles::Profile;

/// argv for `ssh` (without the program) from a saved profile. Every value
/// reaching argv is validated: profiles.json is shared and untrusted.
pub fn ssh_command(profile: &Profile, all: &[Profile]) -> Result<Vec<String>, String> {
    let (user, host, port) = target(profile, "")?;
    let mut argv = vec!["-p".to_string(), port.to_string()];
    if profile.forward_agent {
        argv.push("-A".into());
    }
    // The Flutter app may write "" for "no jump".
    if let Some(id) = profile.jump_profile_id.as_deref().filter(|s| !s.is_empty()) {
        if id == profile.id {
            return Err("jump to itself".into());
        }
        let jump = all.iter().find(|p| p.id == id).ok_or("jump profile not found")?;
        if jump.jump_profile_id.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err("chained jump".into());
        }
        let (ju, jh, jp) = target(jump, "jump ")?;
        let jh = if jh.contains(':') { format!("[{jh}]") } else { jh.to_string() };
        argv.extend(["-J".into(), format!("{ju}@{jh}:{jp}")]);
    }
    argv.extend(["--".into(), format!("{user}@{host}")]);
    Ok(argv)
}

/// Validated (user, host, port). `what` prefixes errors for the jump host.
/// Errors name the field only; the value may hold control characters.
fn target<'a>(p: &'a Profile, what: &str) -> Result<(&'a str, &'a str, u16), String> {
    let err = |f: &str| format!("invalid {what}{f}");
    if !valid_host(&p.host) {
        return Err(err("host"));
    }
    if !valid_user(&p.username) {
        return Err(err("username"));
    }
    let port = match p.port {
        None => 22,
        Some(n) => u16::try_from(n).ok().filter(|&n| n != 0).ok_or_else(|| err("port"))?,
    };
    Ok((&p.username, &p.host, port))
}

fn valid_user(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= 32
        && !u.starts_with('-')
        && u.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn valid_host(h: &str) -> bool {
    if h.is_empty() || h.len() > 253 || h.starts_with('-') {
        return false;
    }
    let name = h.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c));
    let v6 = h.contains(':') && h.chars().all(|c| c.is_ascii_hexdigit() || ":.".contains(c));
    name || v6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, host: &str, user: &str, port: Option<u32>) -> Profile {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "host": host, "username": user, "port": port
        }))
        .unwrap()
    }
    fn cmd(pr: &Profile) -> Result<Vec<String>, String> {
        ssh_command(pr, &[])
    }
    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn valid_profile() {
        assert_eq!(
            cmd(&p("a", "10.0.0.1", "ops", Some(2222))).unwrap(),
            v(&["-p", "2222", "--", "ops@10.0.0.1"])
        );
        assert_eq!(cmd(&p("a", "h", "ops", None)).unwrap()[1], "22");
    }

    #[test]
    fn forward_agent_adds_a() {
        let mut pr = p("a", "h", "ops", None);
        pr.forward_agent = true;
        let c = cmd(&pr).unwrap();
        let (a, sep) = (
            c.iter().position(|x| x == "-A").unwrap(),
            c.iter().position(|x| x == "--").unwrap(),
        );
        assert!(a < sep);
    }

    #[test]
    fn rejects_option_injection() {
        for h in ["-oProxyCommand=x", "a b", "a\nb", "a;b", "$(id)", ""] {
            assert_eq!(cmd(&p("a", h, "ops", None)), Err("invalid host".into()), "{h:?}");
        }
        for u in ["-x", "a@b", ""] {
            assert_eq!(cmd(&p("a", "h", u, None)), Err("invalid username".into()), "{u:?}");
        }
        assert!(cmd(&p("a", &"a".repeat(254), "ops", None)).is_err());
        assert!(cmd(&p("a", "h", &"a".repeat(33), None)).is_err());
    }

    #[test]
    fn rejects_bad_ports() {
        for port in [0, 70000] {
            assert_eq!(cmd(&p("a", "h", "ops", Some(port))), Err("invalid port".into()));
        }
    }

    #[test]
    fn no_option_after_separator() {
        let c = cmd(&p("a", "10.0.0.1", "ops", None)).unwrap();
        let sep = c.iter().position(|x| x == "--").unwrap();
        assert!(c[sep + 1..].iter().all(|x| !x.starts_with('-')));
    }

    #[test]
    fn jump_rules() {
        let mut me = p("me", "10.0.0.1", "ops", None);
        let j = p("j", "10.0.0.9", "j", None);
        me.jump_profile_id = Some("j".into());
        let all = [p("x", "h", "u", None), j];
        assert_eq!(
            ssh_command(&me, &all).unwrap(),
            v(&["-p", "22", "-J", "j@10.0.0.9:22", "--", "ops@10.0.0.1"])
        );
        let mut s = p("me", "h", "ops", None);
        s.jump_profile_id = Some("me".into());
        assert_eq!(ssh_command(&s, &[p("me", "h", "ops", None)]), Err("jump to itself".into()));
        me.jump_profile_id = Some("nope".into());
        assert_eq!(ssh_command(&me, &all), Err("jump profile not found".into()));
        let mut j2 = p("j", "10.0.0.9", "j", None);
        j2.jump_profile_id = Some("x".into());
        me.jump_profile_id = Some("j".into());
        assert_eq!(ssh_command(&me, &[j2]), Err("chained jump".into()));
        me.jump_profile_id = Some(String::new());
        assert!(!ssh_command(&me, &all).unwrap().contains(&"-J".to_string()));
    }

    #[test]
    fn jump_validation() {
        let mut me = p("me", "h", "ops", None);
        me.jump_profile_id = Some("j".into());
        assert_eq!(ssh_command(&me, &[p("j", "-x", "j", None)]), Err("invalid jump host".into()));
        assert_eq!(ssh_command(&me, &[p("j", "h", "-x", None)]), Err("invalid jump username".into()));
        assert_eq!(ssh_command(&me, &[p("j", "h", "j", Some(0))]), Err("invalid jump port".into()));
    }

    #[test]
    fn ipv6_host_and_jump() {
        let c = cmd(&p("a", "::1", "ops", None)).unwrap();
        assert_eq!(c.last().unwrap(), "ops@::1");
        let mut me = p("me", "h", "ops", None);
        me.jump_profile_id = Some("j".into());
        let c = ssh_command(&me, &[p("j", "fe80::1", "j", None)]).unwrap();
        assert!(c.windows(2).any(|w| w == ["-J", "j@[fe80::1]:22"]));
        assert!(cmd(&p("a", "zz::1", "ops", None)).is_err());
    }
}
