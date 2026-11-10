//! The cloud a context belongs to, its words for the same concepts, and
//! what c9s can show for it.
//!
//! c9s shows one context at a time (like k9s with clusters). Each cloud is
//! a [`crate::loader::Fetcher`] answering the same generic requests (the
//! tree, accounts, containers, resources, a resource's document); the
//! views use [`Terms`] so they speak that cloud's language. Features only
//! one cloud has (Azure Policy, RBAC, Entra ID) are gated by
//! [`Provider::supports`].

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Azure,
    Gcp,
    Aws,
}

/// A cloud's words for the levels c9s browses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terms {
    /// What a context is (`:ctx`): "tenant", "organization", "profile"
    pub context: &'static str,
    /// The tree above accounts: "Management Groups", "Folders", "Organizational Units"
    pub tree: &'static str,
    /// A billing/access boundary: "subscription", "project", "account"
    pub account: &'static str,
    pub accounts: &'static str,
    /// Short form for the status bar: "sub", "project", "account"
    pub account_short: &'static str,
    /// What groups resources inside an account: "resource group", "location", "region"
    pub container: &'static str,
    pub containers: &'static str,
}

/// Features beyond browsing, which only some clouds have here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// Azure Policy: `:policy`, `:compliance`, `:exemption`
    Policy,
    /// Azure RBAC: `:ra`, `:role`, `:access`
    Rbac,
    /// Entra ID directory: `:group`, `:app`
    Directory,
    /// The tree above accounts (`:mg`)
    Tree,
    /// A level between accounts and resources (`:rg`)
    Containers,
    /// Cost Management: `:cost`
    Cost,
}

impl Feature {
    /// How the feature is named in messages.
    pub fn label(self) -> &'static str {
        match self {
            Feature::Policy => "Azure Policy",
            Feature::Rbac => "Azure role assignments",
            Feature::Directory => "Entra ID (groups and applications)",
            Feature::Tree => "The tree above accounts",
            Feature::Containers => "Resource groups",
            Feature::Cost => "Cost",
        }
    }
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Azure, Provider::Gcp, Provider::Aws];

    /// Short lowercase name, as shown in the status bar and config.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Azure => "azure",
            Provider::Gcp => "gcp",
            Provider::Aws => "aws",
        }
    }

    pub fn terms(self) -> Terms {
        match self {
            Provider::Azure => Terms {
                context: "tenant",
                tree: "Management Groups",
                account: "subscription",
                accounts: "Subscriptions",
                account_short: "sub",
                container: "resource group",
                containers: "Resource Groups",
            },
            Provider::Gcp => Terms {
                context: "organization",
                tree: "Folders",
                account: "project",
                accounts: "Projects",
                account_short: "project",
                container: "location",
                containers: "Locations",
            },
            Provider::Aws => Terms {
                context: "profile",
                tree: "Organizational Units",
                account: "account",
                accounts: "Accounts",
                account_short: "account",
                container: "region",
                containers: "Regions",
            },
        }
    }

    /// Whether c9s can show `feature` for this cloud.
    pub fn supports(self, feature: Feature) -> bool {
        match feature {
            Feature::Policy
            | Feature::Rbac
            | Feature::Directory
            | Feature::Containers
            | Feature::Cost => self == Provider::Azure,
            // Management groups; GCP folders; AWS organizational units.
            Feature::Tree => true,
        }
    }

    /// Why a command isn't available here, or `None` if it is.
    pub fn unsupported(self, feature: Feature) -> Option<String> {
        (!self.supports(feature)).then(|| {
            if Provider::ALL
                .iter()
                .any(|p| *p != Provider::Azure && p.supports(feature))
            {
                format!(
                    "{} isn't available for {} yet. Switch context with :ctx.",
                    feature.label(),
                    self.as_str()
                )
            } else {
                format!(
                    "{} is Azure only: this context is {}. Switch to an Azure context with :ctx.",
                    feature.label(),
                    self.as_str()
                )
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cloud_has_its_words_and_azure_its_extras() {
        assert_eq!(Provider::default(), Provider::Azure);
        assert_eq!(Provider::Azure.terms().accounts, "Subscriptions");
        assert_eq!(Provider::Gcp.terms().account, "project");
        assert_eq!(Provider::Aws.terms().containers, "Regions");
        for p in Provider::ALL {
            let t = p.terms();
            assert!(
                !t.account.is_empty() && !t.tree.is_empty(),
                "{}",
                p.as_str()
            );
            assert_eq!(p.supports(Feature::Rbac), p == Provider::Azure);
        }
        assert!(Provider::Azure.unsupported(Feature::Policy).is_none());
        assert!(Provider::Gcp.supports(Feature::Tree), "folders");
        assert!(
            Provider::Aws.supports(Feature::Tree),
            "organizational units"
        );
        let msg = Provider::Gcp.unsupported(Feature::Directory).unwrap();
        assert!(
            msg.contains("Azure only") && msg.contains("gcp") && msg.contains(":ctx"),
            "{msg}"
        );
    }
}
