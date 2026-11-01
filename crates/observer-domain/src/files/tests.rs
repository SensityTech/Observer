use super::*;

fn path(text: &str) -> Vec<String> {
    text.split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn allow(folder: &str, depth: Depth, names: &[&str], rights: Rights) -> Rule {
    Rule {
        effect: Effect::Allow,
        folder: folder.to_owned(),
        depth,
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        rights,
        max_file_size: None,
    }
}

fn deny(folder: &str, depth: Depth, names: &[&str], rights: Rights) -> Rule {
    Rule {
        effect: Effect::Deny,
        ..allow(folder, depth, names, rights)
    }
}

fn decide(groups: &[(i64, &[Rule])], at: &str, right: Right) -> Decision {
    RuleSet {
        groups,
        case: CaseRules::LINUX,
    }
    .decide(&path(at), right)
}

#[test]
fn networks_are_compared_by_their_prefix_and_ipv4_over_ipv6_as_ipv4() {
    let blocks = vec![
        "192.0.2.0/24".to_owned(),
        "2001:db8::/32".to_owned(),
        "198.51.100.7".to_owned(),
    ];
    let allows = |ip: &str| network_allows(&blocks, ip.parse().unwrap());

    assert!(allows("192.0.2.200"));
    assert!(allows("::ffff:192.0.2.9"));
    assert!(!allows("192.0.3.1"));
    assert!(allows("2001:db8:1::5"));
    assert!(!allows("2001:db9::1"));
    assert!(allows("198.51.100.7"));
    assert!(!allows("198.51.100.8"));
    assert!(network_allows(&[], "203.0.113.1".parse().unwrap()));
    assert!(!network_allows(
        &["0.0.0.0/33".to_owned()],
        "203.0.113.1".parse().unwrap()
    ));
    assert!(network_allows(
        &["0.0.0.0/0".to_owned()],
        "203.0.113.1".parse().unwrap()
    ));
}

#[test]
fn the_glob_matcher_takes_brackets_literally() {
    let cases = [
        ("*.lua", "client.lua", true),
        ("*.lua", "client.luac", false),
        ("fxmanifest.lua", "fxmanifest.lua", true),
        ("?.ytd", "a.ytd", true),
        ("?.ytd", "ab.ytd", false),
        ("[maps]", "[maps]", true),
        ("[maps]", "m", false),
        ("[*]", "[cars]", true),
        ("[*]", "cars", false),
        ("*", "", true),
        ("a*b*c", "aXbYbZc", true),
        ("a*b*c", "aXbYbZ", false),
        ("*.yt*", "props.ytd", true),
        ("", "x", false),
    ];
    for (glob, name, expected) in cases {
        assert_eq!(glob_matches(glob, name, false), expected, "{glob} against {name}");
    }
    assert!(!glob_matches("Server.cfg", "server.cfg", false));
    assert!(glob_matches("Server.cfg", "SERVER.CFG", true));
    assert!(glob_matches("*.CFG", "permissions.cfg", true));
    // Names beyond ASCII: `?` and `*` stand for characters, not bytes.
    assert!(glob_matches("caf?", "café", false));
    assert!(glob_matches("*é?", "caféé", false));
    assert!(glob_matches("C*É", "café", true));
    assert!(!glob_matches("caf??", "café", false));
    assert!(glob_matches("*_données_*", "x_données_y", false));
}

#[test]
fn a_deny_in_any_group_overrides_every_allow() {
    let edit = [allow("resources", Depth::Tree, &[], Rights::FULL)];
    let no_sql = [deny(
        "resources",
        Depth::Tree,
        &["*.sql"],
        Rights::of(&[Right::Read]),
    )];
    let groups: [(i64, &[Rule]); 2] = [(1, &edit), (2, &no_sql)];

    assert!(decide(&groups, "resources/shop/client.lua", Right::Read).is_allowed());
    assert_eq!(
        decide(&groups, "resources/shop/dump.sql", Right::Read),
        Decision::Denied {
            rule: RuleRef { group: 2, index: 0 }
        }
    );
    // A deny naming read leaves the changes alone.
    assert!(decide(&groups, "resources/shop/dump.sql", Right::Delete).is_allowed());
}

#[test]
fn a_deny_of_one_change_denies_all_four() {
    let edit = [allow("", Depth::Tree, &[], Rights::FULL)];
    let no_delete = [deny(
        "resources",
        Depth::Tree,
        &["fxmanifest.lua"],
        Rights::of(&[Right::Delete]),
    )];
    let groups: [(i64, &[Rule]); 2] = [(1, &edit), (2, &no_delete)];

    for right in [Right::Create, Right::Modify, Right::Rename, Right::Delete] {
        assert!(
            !decide(&groups, "resources/shop/fxmanifest.lua", right).is_allowed(),
            "{right}"
        );
    }
    assert!(decide(&groups, "resources/shop/fxmanifest.lua", Right::Read).is_allowed());
}

#[test]
fn a_deny_matches_a_folder_on_the_way_by_name() {
    let edit = [allow("resources", Depth::Tree, &[], Rights::FULL)];
    let no_git = [deny("resources", Depth::Tree, &[".git"], Rights::FULL)];
    let groups: [(i64, &[Rule]); 2] = [(1, &edit), (2, &no_git)];

    assert!(!decide(&groups, "resources/shop/.git/config", Right::Read).is_allowed());
    assert!(!decide(&groups, "resources/shop/.git", Right::Rename).is_allowed());
    assert!(decide(&groups, "resources/shop/git.lua", Right::Read).is_allowed());
}

#[test]
fn a_folder_rule_reaches_its_entries_and_a_tree_rule_everything_below() {
    let folder = [allow("resources/[maps]", Depth::Folder, &[], Rights::VIEW)];
    let tree = [allow("resources/[cars]", Depth::Tree, &[], Rights::VIEW)];
    let groups: [(i64, &[Rule]); 2] = [(1, &folder), (2, &tree)];

    assert!(decide(&groups, "resources/[maps]/readme.txt", Right::Read).is_allowed());
    assert!(!decide(&groups, "resources/[maps]/harbour/stream.ytd", Right::Read).is_allowed());
    assert!(decide(&groups, "resources/[cars]/f40/stream/f40.yft", Right::Read).is_allowed());
    assert!(!decide(&groups, "resources/[cars]/f40/stream/f40.yft", Right::Modify).is_allowed());
    assert_eq!(decide(&groups, "server.cfg", Right::Read), Decision::NoRule);
}

#[test]
fn globs_in_the_folder_match_one_component_each() {
    let streams = [allow("resources/*/stream", Depth::Tree, &["*.ytd"], Rights::EDIT)];
    let groups: [(i64, &[Rule]); 1] = [(1, &streams)];

    assert!(decide(&groups, "resources/harbour/stream/dock.ytd", Right::Modify).is_allowed());
    assert!(decide(&groups, "resources/harbour/stream/lod/dock.ytd", Right::Create).is_allowed());
    assert!(!decide(&groups, "resources/[maps]/harbour/stream/dock.ytd", Right::Modify).is_allowed());
    assert!(!decide(&groups, "resources/harbour/stream/dock.lua", Right::Modify).is_allowed());
    // Making a folder is not filtered by names.
    assert!(decide(&groups, "resources/harbour/stream/lod", Right::MakeFolder).is_allowed());
}

#[test]
fn the_largest_size_limit_wins_and_a_rule_without_one_lifts_it() {
    let mut small = allow("resources", Depth::Tree, &[], Rights::EDIT);
    small.max_file_size = Some(1_000);
    let mut large = small.clone();
    large.max_file_size = Some(5_000);
    let unlimited = allow("resources", Depth::Tree, &[], Rights::EDIT);

    let limited: [(i64, &[Rule]); 2] = [
        (1, std::slice::from_ref(&small)),
        (2, std::slice::from_ref(&large)),
    ];
    assert_eq!(
        decide(&limited, "resources/a.lua", Right::Create),
        Decision::Allowed {
            rule: RuleRef { group: 1, index: 0 },
            max_file_size: Some(5_000)
        }
    );
    let lifted: [(i64, &[Rule]); 2] = [
        (1, std::slice::from_ref(&small)),
        (2, std::slice::from_ref(&unlimited)),
    ];
    assert!(matches!(
        decide(&lifted, "resources/a.lua", Right::Create),
        Decision::Allowed {
            max_file_size: None,
            ..
        }
    ));
}

#[test]
fn deny_rules_ignore_case_on_linux_while_allow_rules_do_not() {
    let edit = [allow("resources", Depth::Tree, &["*.lua"], Rights::FULL)];
    let no_cfg = [deny("", Depth::Tree, &["*.cfg"], Rights::FULL)];
    let groups: [(i64, &[Rule]); 2] = [(1, &edit), (2, &no_cfg)];
    let linux = RuleSet {
        groups: &groups,
        case: CaseRules::LINUX,
    };
    let windows = RuleSet {
        groups: &groups,
        case: CaseRules::WINDOWS,
    };

    assert!(
        linux
            .denying(&path("resources/shop/Server.CFG"), Right::Read)
            .is_some()
    );
    assert!(
        !linux
            .decide(&path("resources/shop/client.LUA"), Right::Read)
            .is_allowed()
    );
    assert!(
        windows
            .decide(&path("resources/shop/client.LUA"), Right::Read)
            .is_allowed()
    );
}

#[test]
fn folders_on_the_way_show_but_grant_nothing() {
    let maps = [allow("resources/[maps]/harbour", Depth::Tree, &[], Rights::VIEW)];
    let groups: [(i64, &[Rule]); 1] = [(1, &maps)];
    let rules = RuleSet {
        groups: &groups,
        case: CaseRules::LINUX,
    };

    assert!(rules.on_the_way(&path("")));
    assert!(rules.on_the_way(&path("resources/[maps]")));
    assert!(!rules.may_list(&path("resources")));
    assert!(rules.shows(&path("resources"), true));
    assert!(!rules.shows(&path("resources/[cars]"), true));
    assert!(!rules.shows(&path("server.cfg"), false));
    assert!(rules.may_list(&path("resources/[maps]/harbour/stream")));
    assert!(rules.shows(&path("resources/[maps]/harbour/stream/dock.ytd"), false));
}

#[test]
fn a_tree_rule_lists_every_folder_below_whatever_its_names() {
    let textures = [allow("resources", Depth::Tree, &["*.ytd"], Rights::VIEW)];
    let groups: [(i64, &[Rule]); 1] = [(1, &textures)];
    let rules = RuleSet {
        groups: &groups,
        case: CaseRules::LINUX,
    };

    assert!(rules.may_list(&path("resources/harbour/stream")));
    assert!(rules.shows(&path("resources/harbour/stream/dock.ytd"), false));
    assert!(!rules.shows(&path("resources/harbour/client.lua"), false));
}

#[test]
fn a_deny_of_list_hides_the_entry() {
    let all = [allow("", Depth::Tree, &[], Rights::FULL)];
    let hidden = [deny("", Depth::Tree, &["*.sql"], Rights::of(&[Right::List]))];
    let groups: [(i64, &[Rule]); 2] = [(1, &all), (2, &hidden)];
    let rules = RuleSet {
        groups: &groups,
        case: CaseRules::LINUX,
    };

    assert!(!rules.shows(&path("backups/dump.sql"), false));
    assert!(rules.shows(&path("backups/readme.txt"), false));
}

#[test]
fn reach_counts_only_literal_asset_extensions() {
    let view = [allow(
        "resources",
        Depth::Tree,
        &[],
        Rights::of(&[Right::List, Right::Read, Right::MakeFolder]),
    )];
    assert_eq!(reach(&view), Reach::Read);
    let textures = [allow(
        "resources",
        Depth::Tree,
        &["*.ytd", "*.PNG", "logo.webp"],
        Rights::EDIT,
    )];
    assert_eq!(reach(&textures), Reach::Assets);
    for names in [
        &["*.yt*"][..],
        &["*"],
        &[],
        &["*.lua"],
        &["*.svg"],
        &["*.html"],
        &["*.meta"],
    ] {
        let rules = [allow("resources", Depth::Tree, names, Rights::EDIT)];
        assert_eq!(reach(&rules), Reach::Code, "{names:?}");
    }
    // A deny takes nothing away from a group's reach, and gives none either.
    let mixed = [
        allow("resources", Depth::Tree, &["*.ytd"], Rights::EDIT),
        deny("resources", Depth::Tree, &["*.lua"], Rights::FULL),
    ];
    assert_eq!(reach(&mixed), Reach::Assets);
}

#[test]
fn normalising_clamps_at_the_root_and_refuses_what_windows_would_alias() {
    assert_eq!(
        normalise("/resources/./shop/../../../server.cfg").unwrap(),
        path("server.cfg")
    );
    assert_eq!(normalise("..").unwrap(), Vec::<String>::new());
    assert_eq!(normalise("resources//[maps]/").unwrap(), path("resources/[maps]"));
    for refused in ["a\0b", "a\\b", "line\nbreak"] {
        assert_eq!(normalise(refused), Err(PathError::Character), "{refused:?}");
    }
    for refused in [
        "server.cfg:hidden",
        "trailing.",
        "trailing ",
        "CON",
        "con.txt",
        "Nul.lua",
        "COM1",
        "lpt9.log",
        "COM¹",
        "CONIN$",
        "aux .txt",
    ] {
        assert_eq!(
            normalise(&format!("resources/{refused}")),
            Err(PathError::Name),
            "{refused:?}"
        );
    }
    for kept in ["console.lua", "com10", "nullable.lua", "[maps]", "a.b.c"] {
        assert!(normalise(kept).is_ok(), "{kept:?}");
    }
    assert_eq!(normalise(&"a/".repeat(65)), Err(PathError::TooLong));
    assert_eq!(normalise(&"a".repeat(256)), Err(PathError::TooLong));
    assert_eq!(
        normalise(&format!("{}/", "abcdefghi/".repeat(103))),
        Err(PathError::TooLong)
    );
}

#[test]
fn no_normalised_path_climbs_above_the_root() {
    let parts = ["..", ".", "a", "", "b", "..", "c"];
    // Every path of up to six parts drawn from those.
    for length in 0..=6u32 {
        for mut code in 0..parts.len().pow(length) {
            let mut raw = String::new();
            for _ in 0..length {
                raw.push('/');
                raw.push_str(parts[code % parts.len()]);
                code /= parts.len();
            }
            let normalised = normalise(&raw).unwrap();
            assert!(
                normalised
                    .iter()
                    .all(|part| part != ".." && part != "." && !part.is_empty()),
                "{raw}"
            );
        }
    }
}

#[test]
fn short_alias_shapes_are_never_created() {
    for alias in ["SERVER~1.CFG", "abcdef~12", "a~1.txt", "PROGRA~1"] {
        assert!(!creatable(alias), "{alias}");
    }
    for kept in [
        "server.cfg",
        "notes~.txt",
        "a~b.txt",
        "longername~1.txt",
        "x~1.lua5",
        "~1",
    ] {
        assert!(creatable(kept), "{kept}");
    }
}

#[test]
fn names_refused_anywhere_match_without_case() {
    assert!(refused_by_name(&path("resources/shop/.git/config")));
    assert!(refused_by_name(&path("resources/shop/.ENV")));
    assert!(refused_by_name(&path("keys/server.PEM")));
    assert!(refused_by_name(&path(
        "resources/shop/.observer-upload-ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    )));
    assert!(refused_by_name(&path("txData/default/config.json")));
    assert!(!refused_by_name(&path("resources/shop/environment.lua")));
}

#[test]
fn rules_refuse_double_stars_paths_and_sizes_on_denies() {
    let mut rule = allow("resources/**", Depth::Tree, &[], Rights::VIEW);
    assert_eq!(rule.check(), Err(RuleField::Folder));
    rule.folder = "resources/../txData".to_owned();
    assert_eq!(rule.check(), Err(RuleField::Folder));
    rule.folder = "resources/[maps]".to_owned();
    assert_eq!(rule.check(), Ok(()));
    rule.names = vec!["a/b".to_owned()];
    assert_eq!(rule.check(), Err(RuleField::Names));
    rule.names.clear();
    rule.rights = Rights::NONE;
    assert_eq!(rule.check(), Err(RuleField::Rights));
    let mut sized = deny("", Depth::Tree, &[], Rights::FULL);
    sized.max_file_size = Some(10);
    assert_eq!(sized.check(), Err(RuleField::MaxFileSize));
}

#[test]
fn rights_travel_as_a_list_of_names() {
    let json = serde_json::to_string(&Rights::UPLOAD).unwrap();
    assert_eq!(json, r#"["list","create","modify","makeFolder"]"#);
    let back: Rights = serde_json::from_str(&json).unwrap();
    assert_eq!(back, Rights::UPLOAD);
}

#[test]
fn only_the_owner_gives_to_a_pipeline_or_edits_their_own_access() {
    use crate::delegation::Account;
    use crate::permissions::Permissions as P;

    let owner = Account {
        id: 1,
        is_owner: true,
        permissions: P::NONE,
    };
    let manager = Account {
        id: 2,
        is_owner: false,
        permissions: P::FILES_MANAGE | P::CONSOLE_VIEW,
    };
    let owner_access = AccessHolder {
        user_id: 1,
        is_owner: true,
        permissions: P::NONE,
    };
    let own = AccessHolder {
        user_id: 2,
        is_owner: false,
        permissions: manager.permissions,
    };
    let weaker = AccessHolder {
        user_id: 3,
        is_owner: false,
        permissions: P::CONSOLE_VIEW,
    };
    let stronger = AccessHolder {
        user_id: 4,
        is_owner: false,
        permissions: P::SERVER_CONTROL,
    };

    assert_eq!(
        may_manage(&owner, Some(&owner_access), AccessChange::Give),
        Ok(())
    );
    assert_eq!(may_manage(&owner, None, AccessChange::Give), Ok(()));
    assert_eq!(
        may_manage(&manager, None, AccessChange::Give),
        Err(ErrorCode::OwnerOnly)
    );
    assert_eq!(may_manage(&manager, None, AccessChange::TakeAway), Ok(()));
    assert_eq!(
        may_manage(&manager, Some(&own), AccessChange::TakeAway),
        Err(ErrorCode::SelfEdit)
    );
    assert_eq!(
        may_manage(&manager, Some(&owner_access), AccessChange::TakeAway),
        Err(ErrorCode::OwnerProtected)
    );
    assert_eq!(may_manage(&manager, Some(&weaker), AccessChange::Give), Ok(()));
    assert_eq!(
        may_manage(&manager, Some(&stronger), AccessChange::Give),
        Err(ErrorCode::Hierarchy)
    );
    assert_eq!(
        may_manage(&manager, Some(&stronger), AccessChange::TakeAway),
        Ok(())
    );
}

#[test]
fn removing_a_group_lifts_a_denial_only_when_a_kept_group_allows_the_right() {
    let no_cfg = [deny("", Depth::Tree, &["*.cfg"], Rights::of(&[Right::Modify]))];
    let readers = [allow("resources", Depth::Tree, &[], Rights::VIEW)];
    let editors = [allow("resources", Depth::Tree, &[], Rights::EDIT)];

    assert!(!lifts_denial(&no_cfg, &[&readers]));
    // A deny of one change denies the four, so an allowed create is lifted too.
    assert!(lifts_denial(&no_cfg, &[&readers, &editors]));
    assert!(!lifts_denial(&readers, &[&editors]));
}

#[test]
fn a_deny_on_a_folder_covers_everything_inside_it() {
    let view = [allow("", Depth::Tree, &[], Rights::VIEW)];
    let private = [deny(
        "resources",
        Depth::Folder,
        &["private"],
        Rights::of(&[Right::Read, Right::List]),
    )];
    let groups: [(i64, &[Rule]); 2] = [(1, &view), (2, &private)];

    assert!(!decide(&groups, "resources/private/keys.lua", Right::Read).is_allowed());
    assert!(!decide(&groups, "resources/private/sub/deep.lua", Right::Read).is_allowed());
    let rules = RuleSet {
        groups: &groups,
        case: CaseRules::LINUX,
    };
    assert!(!rules.may_list(&path("resources/private/sub")));
    assert!(decide(&groups, "resources/public/keys.lua", Right::Read).is_allowed());
}

#[test]
fn built_in_refusals_take_every_cfg_the_cache_and_observers_own_folders() {
    let observer = vec![path("observer/data")];
    assert!(built_in_refusal(&path("server.cfg"), &observer));
    assert!(built_in_refusal(
        &path("resources/vmenu/config/permissions.CFG"),
        &observer
    ));
    assert!(built_in_refusal(&path("cache/files/x"), &observer));
    assert!(built_in_refusal(
        &path("Observer/Data/observer.sqlite3"),
        &observer
    ));
    assert!(built_in_refusal(&path("resources/.git/config"), &observer));
    assert!(!built_in_refusal(&path("resources/cache/x.lua"), &observer));
    assert!(!built_in_refusal(&path("observer/readme.txt"), &observer));
    assert!(!built_in_refusal(&path("resources/config.lua"), &observer));
}
