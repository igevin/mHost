#[derive(Debug, Clone, PartialEq)]
pub enum TrayMenuAction {
    SwitchProfile(String),
    RefreshRules,
    OpenWindow,
    Quit,
    AdBlock,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileMenuItem {
    pub profile_id: String,
    pub name: String,
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuUpdateKind {
    CheckOnly,
    Rebuild,
}

pub fn resolve_menu_action(menu_id: &str) -> TrayMenuAction {
    match menu_id {
        id if id.starts_with("profile.") => {
            TrayMenuAction::SwitchProfile(id.strip_prefix("profile.").unwrap().to_string())
        }
        "refresh_rules" => TrayMenuAction::RefreshRules,
        "open_window" => TrayMenuAction::OpenWindow,
        "quit" => TrayMenuAction::Quit,
        "adblock" => TrayMenuAction::AdBlock,
        _ => TrayMenuAction::Unknown,
    }
}

pub fn build_profile_menu_items(profiles: &[(String, String, bool)]) -> Vec<ProfileMenuItem> {
    profiles
        .iter()
        .map(|(profile_id, name, checked)| ProfileMenuItem {
            profile_id: profile_id.clone(),
            name: name.clone(),
            checked: *checked,
        })
        .collect()
}

pub fn build_tooltip_text(enabled_profile_name: Option<&str>) -> String {
    match enabled_profile_name {
        Some(name) => format!("mHost - {} 已启用", name),
        None => "mHost - 未启用".to_string(),
    }
}

pub fn determine_menu_update_kind(old_profile_ids: &[String], new_profile_ids: &[String]) -> MenuUpdateKind {
    if old_profile_ids == new_profile_ids {
        MenuUpdateKind::CheckOnly
    } else {
        MenuUpdateKind::Rebuild
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_menu_action() {
        let cases = vec![
            ("uuid_profile", "profile.550e8400-e29b-41d4-a716-446655440000",
             TrayMenuAction::SwitchProfile("550e8400-e29b-41d4-a716-446655440000".into())),
            ("refresh", "refresh_rules", TrayMenuAction::RefreshRules),
            ("open_window", "open_window", TrayMenuAction::OpenWindow),
            ("quit", "quit", TrayMenuAction::Quit),
            ("adblock", "adblock", TrayMenuAction::AdBlock),
            ("unknown", "unknown_id", TrayMenuAction::Unknown),
            ("empty", "", TrayMenuAction::Unknown),
            ("profile_prefix_only", "profile.", TrayMenuAction::SwitchProfile("".into())),
        ];
        for (name, input, expected) in cases {
            let result = resolve_menu_action(input);
            assert_eq!(result, expected, "case: {}", name);
        }
    }

    #[test]
    fn test_build_profile_menu_items() {
        let cases = vec![
            (vec![], vec![]),
            (
                vec![(
                    "p1".to_string(),
                    "Profile 1".to_string(),
                    false,
                )],
                vec![ProfileMenuItem {
                    profile_id: "p1".to_string(),
                    name: "Profile 1".to_string(),
                    checked: false,
                }],
            ),
            (
                vec![(
                    "p1".to_string(),
                    "Profile 1".to_string(),
                    true,
                )],
                vec![ProfileMenuItem {
                    profile_id: "p1".to_string(),
                    name: "Profile 1".to_string(),
                    checked: true,
                }],
            ),
            (
                vec![
                    ("p1".to_string(), "Dev".to_string(), false),
                    ("p2".to_string(), "Prod".to_string(), true),
                    ("p3".to_string(), "Test".to_string(), false),
                ],
                vec![
                    ProfileMenuItem {
                        profile_id: "p1".to_string(),
                        name: "Dev".to_string(),
                        checked: false,
                    },
                    ProfileMenuItem {
                        profile_id: "p2".to_string(),
                        name: "Prod".to_string(),
                        checked: true,
                    },
                    ProfileMenuItem {
                        profile_id: "p3".to_string(),
                        name: "Test".to_string(),
                        checked: false,
                    },
                ],
            ),
        ];

        for (input, expected) in cases {
            let actual = build_profile_menu_items(&input);
            assert_eq!(
                actual, expected,
                "build_profile_menu_items({:?}) expected {:?}, got {:?}",
                input, expected, actual
            );
        }
    }

    #[test]
    fn test_build_tooltip_text() {
        let cases = vec![
            (
                Some("Development"),
                "mHost - Development 已启用".to_string(),
            ),
            (None, "mHost - 未启用".to_string()),
        ];

        for (input, expected) in cases {
            let actual = build_tooltip_text(input);
            assert_eq!(
                actual, expected,
                "build_tooltip_text({:?}) expected {:?}, got {:?}",
                input, expected, actual
            );
        }
    }

    #[test]
    fn test_determine_menu_update_kind() {
        let cases = vec![
            (
                vec!["a".to_string(), "b".to_string()],
                vec!["a".to_string(), "b".to_string()],
                MenuUpdateKind::CheckOnly,
            ),
            (
                vec!["a".to_string(), "b".to_string()],
                vec!["b".to_string(), "a".to_string()],
                MenuUpdateKind::Rebuild,
            ),
            (
                vec!["a".to_string()],
                vec!["a".to_string(), "b".to_string()],
                MenuUpdateKind::Rebuild,
            ),
            (
                vec!["a".to_string(), "b".to_string()],
                vec!["a".to_string()],
                MenuUpdateKind::Rebuild,
            ),
            (vec![], vec![], MenuUpdateKind::CheckOnly),
            (
                vec![],
                vec!["a".to_string()],
                MenuUpdateKind::Rebuild,
            ),
            (
                vec!["a".to_string()],
                vec![],
                MenuUpdateKind::Rebuild,
            ),
        ];

        for (old, new, expected) in cases {
            let actual = determine_menu_update_kind(&old, &new);
            assert_eq!(
                actual, expected,
                "determine_menu_update_kind({:?}, {:?}) expected {:?}, got {:?}",
                old, new, expected, actual
            );
        }
    }
}
