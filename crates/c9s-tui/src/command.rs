use c9s_core::query::ResourceQuery;

/// Command mode parser and executor
///
/// Handles k9s-like colon commands for quick navigation:
/// Command names are always singular (k9s style); plural forms are not accepted.
/// - `:mg` / `:managementgroup` - Go to the management group tree
/// - `:sub` / `:subscription` - Go to subscriptions view
/// - `:rg` - Resource groups where you are (current subscription or management
///   group; all subscriptions if there is no context)
/// - `:rg <sub>` / `:resourcegroup <sub>` - Go to resource groups for a subscription
/// - `:rg all` - Go to resource groups across all subscriptions
/// - `:res <sub>` / `:resource <sub>` - Go to resources for a subscription
/// - `:res all` - Go to resources across all subscriptions
/// - `:res [all|<sub>] type=… loc=… tag=…` - Resources narrowed by filters
/// - `:type [all|<sub>] [filters]` - Resource counts per type
/// - `:tag [all|<sub>] [filters]` - Tag key/value counts
#[derive(Debug, Clone)]
pub struct Command {
    pub kind: CommandKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandKind {
    /// Navigate to the management group tree
    ManagementGroups,
    /// Navigate to subscriptions view
    Subscriptions,
    /// Navigate to resource groups view (`loc=`/`tag=` filters)
    ResourceGroups { scope: Scope, query: ResourceQuery },
    /// Navigate to resources view
    Resources { scope: Scope, query: ResourceQuery },
    /// Resource counts per type
    ResourceTypes { scope: Scope, query: ResourceQuery },
    /// Tag key/value counts
    Tags { scope: Scope, query: ResourceQuery },
    /// Policy assignments that apply at a scope, with compliance
    Policy { scope: Scope },
    /// Role assignments that matter at a scope (a management group or
    /// subscription by name, or `all`)
    RoleAssignments { scope: Scope },
    /// Role definitions, built-in and custom
    RoleDefinitions,
    /// Directory groups, optionally narrowed by name
    Groups(Option<String>),
    /// Switch the colour theme (`None`: list them)
    Theme(Option<String>),
    /// Applications (registrations and enterprise apps), optionally
    /// narrowed by name or app ID
    Apps(Option<String>),
    /// One principal's access across the tenant (name, sign-in name, app
    /// ID or object ID)
    Access(String),
    /// Policy exemptions at or below a scope (a management group or
    /// subscription by name, or `all`)
    Exemption { scope: Scope },
    /// Subscriptions ranked by non-compliant resources (a management group
    /// by name, or `all`)
    Compliance { scope: Scope },
    /// What a scope cost (a management group or subscription by name, or
    /// `all`: the tenant root group)
    Cost { scope: Scope },
    /// Tenants: `None` lists them; `Some("all")` or a tenant name/ID switches
    Context(Option<String>),
    /// A known command with bad arguments (the message says why)
    Invalid(String),
    /// Unknown/invalid command
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    /// No scope given: where the user is (the current or highlighted
    /// subscription or management group), else all subscriptions
    Current,
    /// All subscriptions
    All,
    /// The enabled subscriptions below a management group (from the context)
    Group(String),
    /// Specific subscription by name or ID
    Subscription(String),
}

impl Scope {
    /// Whether the scope spans several subscriptions (all, or a group).
    pub fn is_cross_subscription(&self) -> bool {
        matches!(self, Scope::All | Scope::Group(_))
    }
}

impl CommandKind {
    /// The feature beyond browsing a command needs, if any (so it can say
    /// "Azure only" under another cloud).
    pub fn feature(&self) -> Option<c9s_core::provider::Feature> {
        use c9s_core::provider::Feature;
        match self {
            CommandKind::Policy { .. }
            | CommandKind::Compliance { .. }
            | CommandKind::Exemption { .. } => Some(Feature::Policy),
            CommandKind::RoleAssignments { .. }
            | CommandKind::RoleDefinitions
            | CommandKind::Access(_) => Some(Feature::Rbac),
            CommandKind::Groups(_) | CommandKind::Apps(_) => Some(Feature::Directory),
            CommandKind::ManagementGroups => Some(Feature::Tree),
            CommandKind::ResourceGroups { .. } => Some(Feature::Containers),
            CommandKind::Cost { .. } => Some(Feature::Cost),
            _ => None,
        }
    }
}

impl Command {
    /// Parse a command string
    pub fn parse(input: &str) -> Self {
        let input = input.trim();

        // Remove leading colon if present
        let input = input.strip_prefix(':').unwrap_or(input);

        let parts: Vec<&str> = input.split_whitespace().collect();

        if parts.is_empty() {
            return Self {
                kind: CommandKind::Unknown(String::new()),
            };
        }

        let cmd = parts[0].to_lowercase();

        match cmd.as_str() {
            "policy" => Self {
                kind: CommandKind::Policy {
                    scope: match parts.get(1) {
                        None => Scope::Current,
                        Some(t) if t.eq_ignore_ascii_case("all") => Scope::All,
                        Some(_) => Scope::Subscription(parts[1..].join(" ")),
                    },
                },
            },
            "ra" | "roleassignment" | "role-assignment" => Self {
                kind: CommandKind::RoleAssignments {
                    scope: match parts.get(1) {
                        None => Scope::Current,
                        Some(t) if t.eq_ignore_ascii_case("all") => Scope::All,
                        Some(_) => Scope::Subscription(parts[1..].join(" ")),
                    },
                },
            },
            "app" | "application" | "enterpriseapp" => Self {
                kind: CommandKind::Apps(Some(parts[1..].join(" ")).filter(|name| !name.is_empty())),
            },
            "theme" => Self {
                kind: CommandKind::Theme(
                    Some(parts[1..].join(" ")).filter(|name| !name.is_empty()),
                ),
            },
            "group" => Self {
                kind: CommandKind::Groups(
                    Some(parts[1..].join(" ")).filter(|name| !name.is_empty()),
                ),
            },
            "access" => Self {
                kind: match parts[1..].join(" ") {
                    q if q.is_empty() => CommandKind::Invalid(
                        ":access needs a principal: a name, sign-in name, app ID or \
                         object ID (or press p on a row of :ra)"
                            .into(),
                    ),
                    q => CommandKind::Access(q),
                },
            },
            "role" | "roledefinition" | "role-definition" => Self {
                kind: CommandKind::RoleDefinitions,
            },
            "exemption" => Self {
                kind: CommandKind::Exemption {
                    scope: match parts.get(1) {
                        None => Scope::Current,
                        Some(t) if t.eq_ignore_ascii_case("all") => Scope::All,
                        Some(_) => Scope::Subscription(parts[1..].join(" ")),
                    },
                },
            },
            "cost" => Self {
                kind: CommandKind::Cost {
                    scope: match parts.get(1) {
                        None => Scope::Current,
                        Some(t) if t.eq_ignore_ascii_case("all") => Scope::All,
                        // A management group or subscription by name.
                        Some(_) => Scope::Subscription(parts[1..].join(" ")),
                    },
                },
            },
            "compliance" => Self {
                kind: CommandKind::Compliance {
                    scope: match parts.get(1) {
                        None => Scope::Current,
                        Some(t) if t.eq_ignore_ascii_case("all") => Scope::All,
                        // A management group (or subscription) by name.
                        Some(_) => Scope::Subscription(parts[1..].join(" ")),
                    },
                },
            },
            "ctx" | "context" | "tenant" => Self {
                kind: CommandKind::Context(
                    Some(parts[1..].join(" ")).filter(|arg| !arg.is_empty()),
                ),
            },
            "mg" | "managementgroup" | "management-group" => Self {
                kind: CommandKind::ManagementGroups,
            },
            "sub" | "subscription" => Self {
                kind: CommandKind::Subscriptions,
            },
            "rg" | "resourcegroup" | "resource-group" | "res" | "resource" | "type" | "tag" => {
                let (filters, scope_terms): (Vec<&str>, Vec<&str>) =
                    parts[1..].iter().partition(|t| ResourceQuery::is_filter(t));
                let query = match ResourceQuery::parse(filters) {
                    Ok(query) => query,
                    Err(e) => {
                        return Self {
                            kind: CommandKind::Invalid(format!(":{cmd}: {e}")),
                        }
                    }
                };
                let scope = match scope_terms.first() {
                    None => Scope::Current,
                    Some(t) if t.eq_ignore_ascii_case("all") => Scope::All,
                    Some(_) => Scope::Subscription(scope_terms.join(" ")),
                };
                let kind = match cmd.as_str() {
                    "type" => CommandKind::ResourceTypes { scope, query },
                    "tag" => CommandKind::Tags { scope, query },
                    "res" | "resource" => CommandKind::Resources { scope, query },
                    _ if !query.types.is_empty() => CommandKind::Invalid(format!(
                        ":{cmd}: type= filters resources, not resource groups \
                         (use :res or :type; :rg takes loc= and tag=)"
                    )),
                    _ => CommandKind::ResourceGroups { scope, query },
                };
                Self { kind }
            }
            _ => Self {
                kind: CommandKind::Unknown(input.to_string()),
            },
        }
    }

    /// Check if this is a valid command
    pub fn is_valid(&self) -> bool {
        !matches!(self.kind, CommandKind::Unknown(_) | CommandKind::Invalid(_))
    }

    /// Get a user-friendly error message for invalid commands
    pub fn error_message(&self) -> Option<String> {
        match &self.kind {
            CommandKind::Unknown(cmd) if !cmd.is_empty() => {
                Some(format!(
                    "Unknown command: '{}'\n\nAvailable commands:\n  :policy [all|<sub>] - Policy assignments and compliance\n  :compliance [all|<mg>] - Subscriptions by non-compliant resources\n  :exemption [all|<mg>|<sub>] - Policy exemptions and when they expire\n  :ra [all|<mg>|<sub>] - Role assignments (inherited, here, below)\n  :role - Role definitions, built-in and custom\n  :access <principal> - Everything one user, group or app can do\n  :group [name] - Directory groups and their members\n  :app [name] - App registrations and enterprise applications\n  :theme [name] - Switch the colour theme\n  :ctx [all|<tenant>] - Tenants\n  :mg - Management group tree\n  :sub - Subscriptions\n  :rg [all|<sub>] [loc=… tag=…] - Resource groups\n  :res [all|<sub>] [type=… loc=… tag=…] - Resources\n  :type [all|<sub>] - Resource counts per type\n  :tag [all|<sub>] - Tags and how many resources carry them",
                    cmd
                ))
            }
            CommandKind::Unknown(_) => Some("No command entered".to_string()),
            CommandKind::Invalid(message) => Some(message.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_management_groups() {
        for input in [":mg", "managementgroup", ":management-group"] {
            assert_eq!(Command::parse(input).kind, CommandKind::ManagementGroups);
        }
        assert!(matches!(
            Command::parse(":mgs").kind,
            CommandKind::Unknown(_)
        ));
    }

    #[test]
    fn test_parse_subscriptions() {
        let cmd = Command::parse(":sub");
        assert_eq!(cmd.kind, CommandKind::Subscriptions);
        assert!(cmd.is_valid());

        let cmd = Command::parse("subscription");
        assert_eq!(cmd.kind, CommandKind::Subscriptions);
    }

    #[test]
    fn test_parse_resource_groups() {
        let cmd = Command::parse(":rg all");
        assert_eq!(
            cmd.kind,
            CommandKind::ResourceGroups {
                scope: Scope::All,
                query: ResourceQuery::default()
            }
        );
        assert!(cmd.is_valid());

        let cmd = Command::parse(":resourcegroup my-subscription");
        assert_eq!(
            cmd.kind,
            CommandKind::ResourceGroups {
                scope: Scope::Subscription("my-subscription".to_string()),
                query: ResourceQuery::default()
            }
        );
    }

    #[test]
    fn test_parse_resources() {
        let cmd = Command::parse(":res all");
        assert_eq!(
            cmd.kind,
            CommandKind::Resources {
                scope: Scope::All,
                query: ResourceQuery::default()
            }
        );
        assert!(cmd.is_valid());

        let cmd = Command::parse(":resource my-subscription");
        assert_eq!(
            cmd.kind,
            CommandKind::Resources {
                scope: Scope::Subscription("my-subscription".to_string()),
                query: ResourceQuery::default()
            }
        );
    }

    #[test]
    fn test_rg_takes_location_and_tag_filters_but_not_type() {
        let CommandKind::ResourceGroups { scope, query } =
            Command::parse(":rg all loc=westeurope tag=env:prod").kind
        else {
            panic!()
        };
        assert_eq!(scope, Scope::All);
        assert_eq!(query.to_string(), "loc=westeurope tag=env:prod");
        let bad = Command::parse(":rg type=vm");
        assert!(bad.error_message().unwrap().contains("not resource groups"));
    }

    #[test]
    fn test_parse_policy() {
        assert_eq!(
            Command::parse(":policy").kind,
            CommandKind::Policy {
                scope: Scope::Current
            }
        );
        assert_eq!(
            Command::parse(":policy all").kind,
            CommandKind::Policy { scope: Scope::All }
        );
        assert!(!Command::parse(":policies").is_valid(), "singular only");
    }

    #[test]
    fn test_parse_ctx() {
        assert_eq!(Command::parse(":ctx").kind, CommandKind::Context(None));
        assert_eq!(
            Command::parse(":ctx all").kind,
            CommandKind::Context(Some("all".into()))
        );
        assert_eq!(
            Command::parse(":ctx Contoso Ltd").kind,
            CommandKind::Context(Some("Contoso Ltd".into()))
        );
    }

    #[test]
    fn test_parse_tag_view() {
        let CommandKind::Tags { scope, query } = Command::parse(":tag all type=vm").kind else {
            panic!()
        };
        assert_eq!(scope, Scope::All);
        assert_eq!(query.types, ["vm"]);
        assert!(!Command::parse(":tags").is_valid(), "singular only");
    }

    #[test]
    fn test_no_scope_means_current_context_and_all_means_all() {
        assert_eq!(
            Command::parse(":rg").kind,
            CommandKind::ResourceGroups {
                scope: Scope::Current,
                query: ResourceQuery::default()
            }
        );
        assert_eq!(
            Command::parse(":rg ALL").kind,
            CommandKind::ResourceGroups {
                scope: Scope::All,
                query: ResourceQuery::default()
            }
        );
        assert!(matches!(
            Command::parse(":res").kind,
            CommandKind::Resources {
                scope: Scope::Current,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_filters_and_type_summary() {
        let cmd = Command::parse(":res my sub type=vm,disks loc=westeurope");
        let CommandKind::Resources { scope, query } = cmd.kind else {
            panic!("{:?}", cmd.kind)
        };
        assert_eq!(scope, Scope::Subscription("my sub".into()));
        assert_eq!(query.types, ["vm", "disks"]);
        assert_eq!(query.locations, ["westeurope"]);

        let cmd = Command::parse(":res tag=env:prod");
        assert!(matches!(
            cmd.kind,
            CommandKind::Resources {
                scope: Scope::Current,
                ..
            }
        ));

        assert_eq!(
            Command::parse(":type").kind,
            CommandKind::ResourceTypes {
                scope: Scope::Current,
                query: ResourceQuery::default()
            }
        );
        assert!(!Command::parse(":types").is_valid(), "singular only");

        let bad = Command::parse(":res size=big");
        assert!(!bad.is_valid());
        assert!(bad.error_message().unwrap().contains("unknown filter"));
    }

    #[test]
    fn test_plural_commands_are_rejected() {
        for input in [
            ":subs",
            ":subscriptions",
            ":rgs all",
            ":resourcegroups",
            ":resource-groups",
            ":resources all",
        ] {
            let cmd = Command::parse(input);
            assert!(
                matches!(cmd.kind, CommandKind::Unknown(_)),
                "{input} should be rejected"
            );
        }
    }

    #[test]
    fn test_parse_unknown() {
        let cmd = Command::parse(":invalid");
        assert!(matches!(cmd.kind, CommandKind::Unknown(_)));
        assert!(!cmd.is_valid());
        assert!(cmd.error_message().is_some());
    }

    #[test]
    fn test_parse_empty() {
        let cmd = Command::parse("");
        assert!(matches!(cmd.kind, CommandKind::Unknown(_)));
        assert!(!cmd.is_valid());
    }
}
