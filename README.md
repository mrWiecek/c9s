# c9s

*Canines for clouds.* A fast, keyboard-first terminal UI for Azure, in the
spirit of k9s. Formerly **az9s**: see [Upgrading from az9s](#upgrading-from-az9s).

![c9s: k9s for clouds](assets/c9s-banner.png)

**A fast, keyboard-first terminal UI for operating Azure, built for platform engineers.**

Browse every subscription, resource group and resource from the terminal, filter
instantly, and (soon) act on resources and check Azure Policy compliance, the way k9s
works for Kubernetes.

> ⚠️ **Status: early development.** The app browses resources across
> all your subscriptions with Azure Resource Graph and never freezes while loading.
> It is read-only for now. See the [Roadmap](ROADMAP.md).

## Features

| Today | Planned |
|---|---|
| Management group tree → subscriptions → resource groups → resources → detail (Overview, Tags, Policy, JSON) | Command palette, config file, `:ctx` (M2) |
| Resource groups across a whole management group, or all subscriptions (`:rg all`, `:res all`) via Azure Resource Graph | Azure Policy at management-group scope: assignments, compliance, exemptions (M3) |
| Instant views from a local cache, refreshed in the background, with data age in the status bar | Safe actions: VM start/stop, restarts, tags, delete, policy scans (M4) |
| Policy assignments per scope with inheritance and compliance (`:policy`); compliance summary per scope; drill down to non-compliant resources, and each resource's policy states | Compliance roll-up, exemptions (M3) |
| Filters (`type=`, `loc=`, `tag=`), a resource-type summary (`:type`), `/` text filter, `y` copy ID, `o` open in Portal, vim keys | AKS, Key Vault, logs, themes (M5) |

## Requirements

- [Rust](https://rustup.rs) 1.89 or newer and a C compiler (`build-essential` on
  Debian/Ubuntu/WSL; Xcode command line tools on macOS). OpenSSL and `pkg-config`
  are **not** needed: TLS uses rustls.
- [Azure CLI](https://learn.microsoft.com/cli/azure/install-azure-cli), logged in with
  `az login`
- A terminal with colour support. On a light background use `theme = "light"`;
  `theme = "mono"` (or `NO_COLOR=1`) uses no colours at all. The colour-scheme
  themes (`dracula`, `nord`, …) need 24-bit colour, which most current
  terminals have (Windows Terminal, iTerm2, GNOME Terminal, WezTerm, kitty).

## Install

**Windows** (PowerShell, no admin; installs for your user and adds it to PATH):

```powershell
irm https://github.com/mrWiecek/c9s/releases/latest/download/install.ps1 | iex
```

**Linux and WSL** (no sudo; a static binary in `~/.local/bin` that runs on any
distribution):

```bash
curl -fsSL https://github.com/mrWiecek/c9s/releases/latest/download/install.sh | sh
```

Both verify the download against the release's `SHA256SUMS` before
installing. Or use a package:

| | |
|---|---|
| Scoop | `scoop install https://github.com/mrWiecek/c9s/releases/latest/download/c9s.json` |
| Debian, Ubuntu | download `c9s_<version>_amd64.deb` from [Releases](https://github.com/mrWiecek/c9s/releases), then `sudo apt install ./c9s_*_amd64.deb` |
| RHEL, Fedora, SUSE | download `c9s-<version>-1.x86_64.rpm`, then `sudo dnf install ./c9s-*.rpm` |
| macOS | download the `aarch64-apple-darwin` (Apple silicon) or `x86_64-apple-darwin` archive |
| From source | `cargo install --git https://github.com/mrWiecek/c9s --locked c9s` |

Every method, how to verify a download, and what to do when a company
policy blocks it: [docs/INSTALL.md](docs/INSTALL.md).

`c9s --version` shows the installed version and commit. Versions follow
SemVer; see [CHANGELOG.md](CHANGELOG.md) for what changed and
[RELEASING.md](RELEASING.md) for how to roll back.

## Quick start

```bash
az login                                   # authenticate once (c9s uses your Azure CLI login)
c9s                                        # launch
c9s --timings                              # launch, print a timing summary on exit
```

## Keys

| Key | Action |
|---|---|
| `j` / `k`, `↑` / `↓` | Move down / up |
| `g` / `G` | First / last item |
| `Ctrl+D` / `Ctrl+U` | Page down / up |
| `Enter` | Open (in the tree: resource groups of the group or subscription) |
| `Space`, `l`/`→`, `h`/`←` | Tree: open/close a group (`h` on a closed row goes to the parent) |
| `Esc` / `Backspace` | Back, clear filter, or dismiss error |
| `/` | Filter the list; in the detail view, search like Vim (`n`/`N` next/previous) |
| `:` | Command mode; suggestions as you type, `Tab` completes |
| `Ctrl+P` | Go to anything: fuzzy search over commands, subscriptions, management groups and the loaded resource groups and resources |
| `r` | Refresh |
| `y` | Copy the selected item's Azure ID (works over SSH; in tmux enable `set-clipboard`) |
| `c` | Copy the current view: all filtered rows of a list (tab-separated, pastes into a spreadsheet), or the detail view's tab (Overview, Tags, Policy, JSON) |
| `o` | Open the selected item in the Azure Portal |
| `Tab`, `1`-`4` | Detail view: Overview / Tags / Policy / JSON |
| `Enter` (Tags tab) | Every resource in the tenant with the selected tag |
| `Enter` (Policy tab) | The selected policy's definition (initiative or direct, and where it is assigned, show in the tab) |
| `?` | Help |
| `q` / `Ctrl+C` | Quit |

## Commands

| Command | Shows |
|---|---|
| `:ra [all\|<mg>\|<sub>]` | Role assignments that matter there: inherited from management groups above, at the scope, and below it (resource groups, resources). Principal names come from Microsoft Graph; `C`/`B` mark custom and built-in roles. `Enter` opens the role's permissions; `d` the assignment |
| `p` (in `:ra`), `:access <principal>` | Everything one user, group or app can do across the tenant: its own role assignments and those of its groups (nested too), each marked "direct" or "via *group*". `:access` takes a name, sign-in name, app ID or object ID |
| `:app [name]` | Applications: each app registration with its enterprise application (joined by app ID), plus external apps and managed identities (`m` also shows Microsoft's own). Soonest credential expiry per app. `Enter` opens tabs: Overview, Permissions (requested vs granted, admin or user consent, readable names), Exposed API, Owners, Credentials, Assigned users and groups, JSON. `p` shows the app's Azure access |
| `:group [name]` | Directory groups: kind (security, Microsoft 365, distribution), dynamic or assigned, `R` role-assignable. `Enter` lists the members (`t` direct / nested); `Enter` on a member opens a group's members or a user's or app's groups. `p` shows Azure access, `o` the Entra page |
| `:role` | Role definitions, custom first, with how many assignments each has; `Enter` lists who has the role, `d` its permissions (actions, data actions, assignable scopes) |
| `:exemption [all\|<mg>\|<sub>]` | Policy exemptions at or below the scope: category, expiry (red when expired, yellow within 30 days), assignment, scope and reason, soonest to expire first; `Enter` opens one |
| `:compliance [all\|<mg>]` | Subscriptions ranked by non-compliant resources (with % compliant and their management group), worst first; `Enter` opens `:policy` for one. The management group tree shows the same compliance on every group and subscription |
| `:policy [all\|<sub>]` | Policy assignments that apply there (assigned or inherited from management groups), with non-compliant resource counts, worst first, under a compliance summary for the scope (% compliant, non-compliant resources, subscriptions and assignments affected). `Enter`: the assignment's non-compliant resources; `d`: the assignment itself |
| `:ctx [all\|<tenant>]` | Tenants you can use (`Enter` switches), or switch by name, domain or ID; `all` returns to the default login |
| `:mg` | Management group tree (the start view) |
| `:sub` | Subscriptions |
| `:rg [all\|<sub>] [loc=… tag=…]` | Resource groups. Without a scope: where you are (the highlighted or current subscription or management group; all subscriptions if none) |
| `:res [all\|<sub>] [filters]` | Resources, scoped like `:rg`, e.g. `:res all type=vm loc=westeurope tag=env=prod` (`type=` takes aliases like `vm`, `vnet`, `keyvault`, the type's last part, the full type, or a provider). `?` in the app has more examples |
| `:type [all\|<sub>] [filters]` | Resource counts per type; `Enter` lists that type |
| `:tag [all\|<sub>] [filters]` | Tags (key, value, how many resources); `Enter` lists the resources with that tag |

## Development

Requires [just](https://github.com/casey/just).

```bash
just run         # run (release mode, logs off)
just run-debug   # run with debug logs written to c9s.log
just run-timings # run, then print Azure/cache/frame timings on exit
just perf        # rendering benchmark with 100k rows
just ci          # everything CI runs: fmt check, clippy, tests
just --list      # all recipes
```

```
crates/
├── c9s         # binary entry point
├── c9s-core    # state, navigation, SQLite cache
├── c9s-azure   # Azure auth + API client
└── c9s-tui     # ratatui UI
```

How it fits together: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Google Cloud (preview)

Setup and permissions for every cloud: [docs/CLOUDS.md](docs/CLOUDS.md).

c9s also browses Google Cloud: the organization's folders (`:mg`),
projects, their resources (type, location, labels) and each resource's
document; `o` opens the selected item in the Google Cloud console.

1. Install the [Google Cloud CLI](https://cloud.google.com/sdk/docs/install)
   and run `gcloud auth login`.
2. Resources come from Cloud Asset Inventory: enable it once per project
   (`gcloud services enable cloudasset.googleapis.com --project <project>`).
3. In c9s, `:ctx` lists your Azure tenants and gcloud configurations side by
   side; `Enter` (or `:ctx <configuration>`) switches. The status bar shows
   `gcp`; projects open straight onto their resources; `:ctx all` goes back
   to Azure.

Folders need `resourcemanager.folders.list` (e.g. Folder Viewer) on the
organization; without it, `:mg` lists projects under one root.

Not for Google Cloud: the Azure-only views (Policy, role assignments,
Entra ID), which say so.

## AWS (preview)

c9s also browses AWS, one CLI profile at a time: the profile's account,
or every account of its organization (with the organizational units as
`:mg`), their resources (type, region, tags) and each resource's document;
`o` opens the selected resource in the AWS console.

1. Install [AWS CLI v2](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html)
   and sign in (`aws sso login --profile <profile>`, or `aws configure`).
   c9s reads the profiles from `~/.aws/config` and `~/.aws/credentials`
   and asks the CLI for their credentials.
2. Resources come from
   [Resource Explorer](https://docs.aws.amazon.com/resource-explorer/latest/userguide/getting-started-setting-up.html),
   searched in the profile's region (`us-east-1` if it names none). With an
   aggregator index in that region, every region is listed; a search
   returns 1,000 resources at most. Without Resource Explorer, c9s falls
   back to the Tagging API in the profile's region and says so (untagged
   resources may be missing).
3. In c9s, `:ctx` lists AWS profiles next to Azure tenants and gcloud
   configurations; `Enter` (or `:ctx <profile>`) switches. The status bar
   shows `aws · <profile>`; the account opens straight onto its resources;
   `:ctx all` goes back to Azure.

**Organizations.** With a profile in the management account (or a
delegated administrator for AWS Organizations), the accounts list is every
account in the organization and `:mg` shows the organizational units;
`Enter` on a unit lists the resources of every account below it. c9s
reaches another account through a CLI profile that signs in to it (an SSO
profile with that `sso_account_id`, or a `role_arn` in it), else by
assuming `OrganizationAccountAccessRole` there; set `aws_role` in the
config for another role. Accounts it can't reach are reported, the rest
still listed.

Not for AWS: the Azure-only views (Policy, role assignments, Entra ID),
which say so.

## Security

c9s only reads, uses your cloud CLIs' logins and treats everything the
clouds return as untrusted. How, and how to report a vulnerability:
[SECURITY.md](SECURITY.md).

## Upgrading from az9s

c9s v1.0.0 is az9s renamed; nothing else changed.

- The command is `c9s`. To keep typing `az9s`: `alias az9s=c9s` (or a
  `Set-Alias az9s c9s` in PowerShell).
- On first start c9s moves az9s's config file and cache to its own
  directories, so settings and cached lists carry over.
- `AZ9S_CONFIG` and `AZ9S_BACKEND` still work; the new names are
  `C9S_CONFIG` and `C9S_BACKEND`.

## Configuration

Optional. `c9s --init-config` writes a commented file with the defaults;
`c9s --help` shows where it lives (Linux/WSL: `~/.config/c9s/config.toml`;
macOS: `~/Library/Application Support/c9s/config.toml`; Windows:
`%APPDATA%\c9s\config\config.toml`). `--config <path>` or `C9S_CONFIG`
use another file.

```toml
start = "mg"                               # first view: "mg" (tree) or "sub"
default_subscription = "prod-connectivity" # for :rg/:res/:type/:tag without context
tenant = "<tenant-id>"                     # start in this tenant (like :ctx)
theme = "default"                          # see Themes below; NO_COLOR forces mono
refresh_interval = 120                     # auto-refresh every N s (min 30); 0 = off
read_only = true                           # status bar shows RO; actions (M4) disabled
```

A mistake in the file (unknown key, wrong type) doesn't stop c9s: it starts
with the defaults and the status bar names the line and the problem.

### Themes

| Theme | Background | For |
|---|---|---|
| `default` | terminal's | dark terminals |
| `light` | terminal's | light terminals |
| `high-contrast` | terminal's | bright colours only: projectors, low-contrast screens |
| `mono` | terminal's | no colours (also `NO_COLOR=1`) |
| `dracula`, `nord`, `gruvbox`, `catppuccin`, `tokyo-night`, `solarized-dark` | its own, dark | the colour schemes of the same names |
| `solarized-light` | its own, light | |

Set one with `theme = "nord"` in the config file, or try them while c9s runs
with `:theme <name>` (`:theme` lists them; Tab completes). `:theme` lasts until
you quit; the config file keeps it.

## Troubleshooting

| Symptom | Try |
|---|---|
| "Not connected to Azure" | Run `az login`, then press `r` |
| "the Azure CLI (az) was not found" | Install the Azure CLI and check `az --version` runs in the same terminal. On Windows, update to c9s 0.12.1 or later (earlier versions couldn't start `az.cmd`) |
| "No read access to management groups" | Expected without Reader on management groups: c9s starts on the subscription list, and `:sub`, `:rg`, `:res` work. Set `start = "sub"` in the config to skip the tree |
| "Management group tree unavailable" | You need read access on the tenant root management group; `:sub` and `:rg all` still work |
| Status bar says "Resource Graph unavailable … used ARM" | Lists still load, but more slowly; check `just run-debug` logs |
| A resource you just changed looks stale | Resource Graph can lag ARM briefly; press `r`, or run with `C9S_BACKEND=arm` |
| `o` says "No browser could be started" (e.g. WSL) | The Portal link is copied instead; paste it into a browser. On WSL, installing `wslu` (`wslview`) helps, though c9s also tries `cmd.exe` |
| `:ctx` switch fails with "no token for tenant" | Run `az login --tenant <id>` once for that tenant, then switch again |
| Something feels slow | Run `c9s --timings` and include the table in an issue |

## Contributing

Issues and PRs are welcome. Start with [ROADMAP.md](ROADMAP.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). Record user-visible changes under
`[Unreleased]` in [CHANGELOG.md](CHANGELOG.md), and bump the version for code
changes (`just bump patch|minor|major`; see [RELEASING.md](RELEASING.md)).

## License

[Apache License 2.0](LICENSE).
