# Changelog

All notable changes to c9s (az9s before v1.0.0) are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Planned work is tracked in [ROADMAP.md](ROADMAP.md), not here.

Every version below has a git tag (`vX.Y.Z`) and a
[GitHub Release](https://github.com/mrWiecek/c9s/releases) with binaries.
Release process, audit and rollback: [RELEASING.md](RELEASING.md).

## [Unreleased]

## [1.9.1] - 2026-11-09

### Security
- **SQLite updated** (bundled through `rusqlite` 0.41, `libsqlite3-sys`
  0.39): fixes CVE-2026-119375 in SQLite (RUSTSEC-2026-0459, medium), which
  `cargo audit` flagged. The minimum Rust version for building from source
  is now 1.89 (`rusqlite` 0.41 needs it).

### Added
- **Easier installs, for Windows and Linux/WSL above all.** Every release
  now also has:
  - `install.ps1`: `irm …/releases/latest/download/install.ps1 | iex`
    installs c9s for the current user (no admin), adds it to PATH, and
    checks the download against `SHA256SUMS`. Downloads made this way are
    not blocked by SmartScreen.
  - `install.sh`: `curl -fsSL …/install.sh | sh` installs to `~/.local/bin`
    (no sudo), checksum verified; Linux, WSL and macOS.
  - A static Linux build (`x86_64-unknown-linux-musl`) that runs on any
    distribution and WSL whatever its glibc, packaged as `.deb` and `.rpm`
    too.
  - A Scoop manifest (`scoop install …/latest/download/c9s.json`) and winget
    manifests (`winget-manifests.zip`, package `mrWiecek.c9s`).
- [docs/INSTALL.md](docs/INSTALL.md): every way to install, how to verify a
  download, and what to do when security software or a company policy
  blocks c9s.
- CI tests the install scripts on Linux and on Windows (PowerShell 7 and
  5.1), and builds, installs and runs the `.deb`.

## [1.9.0] - 2026-11-08

### Added
- **PIM eligible role assignments.** `:ra`, `p` and `:access` now show
  eligible assignments (Privileged Identity Management) next to the active
  ones, marked *eligible* or *eligible until <date>*; the title counts
  them. `/eligible` lists only them, `/active` the rest. A principal's
  access includes what it is eligible for directly and through its groups,
  so someone eligible for Owner no longer looks less privileged than they
  are. Read from ARM (`roleEligibilityScheduleInstances`) at the tenant
  root, else per subscription; Reader is enough. Ended eligibilities are
  left out. `d` on an eligible row says what it is (PIM eligibility isn't
  a role assignment resource).

## [1.8.0] - 2026-11-08

### Changed
- **Enter takes the highlighted suggestion.** On the command line, with
  suggestions showing for the word being typed, Enter now puts the
  highlighted one in (as Tab does) and runs the command: `:res tmp` Enter
  opens the resources of the matching subscription, no Tab needed. ↑/↓
  still choose another. Enter runs the line as typed when nothing is typed
  after the command (`:res ` = where you are) or the word already is a
  suggestion; a suggestion that still needs a value (`type=`) is filled in
  and waits for it.

## [1.7.0] - 2026-11-06

### Added
- **Copy any field of an application's Overview.** `↑↓`/`j`/`k` select a
  field (object IDs, resource ID, resource group, subscription, redirect
  URIs...); `y` or `c` copies it. The cursor starts on the app ID, so `y`
  still copies it first; `C` copies the whole tab.
- **`o` asks which page to open** when an application has several: the app
  registration, the enterprise application, or a managed identity's Azure
  resource. The selected field's page is offered first; Enter or `1`-`9`
  opens, Esc cancels.

### Fixed
- **Search (`/`) works in an application's tabs**, the JSON tab included:
  matches are highlighted, `n`/`N` step through them, Esc clears it. It
  only worked in the resource detail before.

## [1.6.1] - 2026-11-06

### Security
- **ratatui 0.30 and crossterm 0.29.** ratatui 0.26 pulled in `lru` 0.12
  (RUSTSEC-2026-0002, RUSTSEC-2026-0253: unsoundness) and the unmaintained
  `paste` (RUSTSEC-2024-0436). `cargo audit` now reports no
  vulnerabilities and no warnings. Nothing changes on screen.

## [1.6.0] - 2026-11-06

### Added
- **Managed identities in `:app` say which kind they are and where they
  live.** The list shows *user-assigned MI* or *system-assigned MI*
  (`/managed` still finds both). The Overview tab has a *Managed identity*
  section: the type, the Azure resource (the user-assigned identity, or
  the VM, App Service or other resource a system-assigned identity belongs
  to) with its resource type, subscription, resource group, ID and Azure
  Portal link. `o` on a managed identity opens that resource in the Portal
  instead of the enterprise application. From Microsoft Graph's
  `alternativeNames`; no new permissions.

## [1.5.2] - 2026-11-05

### Security
- **Terminal escape sequences from cloud data are neutralised.** A
  resource name, tag, group name or API error carrying control characters
  (an escape sequence that sets the terminal title, writes the clipboard or
  redraws the screen) or a bidirectional override reached the terminal.
  Every frame is now cleaned before it is drawn.
- **Cloud CLIs are started safely.** `az`, `gcloud` and `aws` are looked
  up on the PATH only (an `az.cmd` in the current directory, e.g. in a
  cloned repository, no longer runs) and started by full path instead of
  through `cmd /C` on Windows, so a tenant, gcloud configuration or AWS
  profile name with `&` or `|` can't run another command. Such names are
  refused on every platform. The Azure login uses c9s's own
  `az account get-access-token` credential for the same reason.
- **No Azure SDK; current TLS.** The Azure SDK (`azure_core` 0.19 could log
  the `Authorization` header) is gone: Azure is called over plain HTTPS
  like the other clouds. reqwest 0.12 brings rustls 0.23.46 and
  rustls-webpki 0.103.16, fixing a TLS key-exchange downgrade and
  certificate validation advisories in the old stack. `cargo audit`
  reports no vulnerabilities.
- **Copying is safe to paste.** `y` and `c` copy without control
  characters; copied tables defuse spreadsheet formulas (`=`, `+`, `-`,
  `@` get a leading `'`).
- **`o` opens only https links to the Azure Portal, Google Cloud console
  and AWS console**, percent-encoded, and on Windows without `cmd.exe`.
- **Tokens stay on the API's host.** Paging links are followed only on the
  same host, ARM IDs from responses must be paths, and an AWS region must
  look like one before it becomes part of a host name.
- **The cache is private** (0700 directory, 0600 files on Unix), and AWS
  credentials no longer show their secret in debug output.
- **Release pipeline.** Actions are pinned to commit SHAs, jobs get
  read-only tokens (only publishing writes), checkouts keep no
  credentials, release builds don't use a shared cache, and every archive
  gets signed build provenance (`gh attestation verify`). CI runs
  `cargo audit`; Dependabot proposes updates weekly.

### Added
- [SECURITY.md](SECURITY.md): how to report a vulnerability, what c9s
  protects and how, and the known, accepted advisories.

## [1.5.1] - 2026-11-04

### Fixed
- Without the Azure CLI, c9s starts on `:ctx` (listing gcloud
  configurations and AWS profiles) instead of an Azure error, so it can be
  used for Google Cloud or AWS alone. When Azure subscriptions fail to
  load, the error points to `:ctx` for the other clouds.
- Help (`?`) covers every cloud: a Clouds section, `:ctx` and `:mg` per
  cloud, and what `y` and `o` do under Google Cloud and AWS.

### Added
- [docs/CLOUDS.md](docs/CLOUDS.md): setup and the read permissions each
  cloud needs (Azure roles, Google Cloud roles and APIs, AWS actions and
  Organizations).

## [1.5.0] - 2026-11-04

### Added
- **AWS Organizations.** With a profile that can read AWS Organizations
  (the management account or a delegated administrator), the accounts list
  is every account in the organization (suspended ones disabled), and
  `:mg` shows the organizational units with their accounts; `Enter` on a
  unit lists the resources of every account below it. Other accounts are
  reached through a CLI profile that signs in to them (an SSO profile's
  `sso_account_id`, or a `role_arn` in them), else by assuming
  `OrganizationAccountAccessRole` in them; `aws_role` in the config names
  another role. Accounts that can't be reached are reported, the rest are
  still listed. Without Organizations access, a profile shows its own
  account as before.
- `o` on an organizational unit opens AWS Organizations in the console.

### Changed
- Under Google Cloud and AWS, a tree that can't be loaded says why and
  shows the projects or accounts (instead of Azure's management group
  message).

## [1.4.0] - 2026-11-04

### Added
- **AWS browsing (preview), one CLI profile at a time.** `:ctx` lists the
  AWS CLI's profiles (from `~/.aws/config` and `~/.aws/credentials`, with
  SSO, role or keys and the region) next to Azure tenants and gcloud
  configurations; `Enter` or `:ctx <profile>` switches to AWS. The
  profile's account is listed; `Enter` lists its resources from Resource
  Explorer (type, region, tags; `/`, `type=`, `tag=` filters work), and
  `Enter` on a resource shows its document. Without Resource Explorer, the
  Tagging API in the profile's region stands in, with a notice. Needs AWS
  CLI v2, signed in. `o` opens a resource in the AWS console by its ARN.
  New crate `c9s-aws` signs requests itself (SigV4), no AWS SDK.

### Changed
- The status bar names the gcloud configuration or AWS profile
  (`gcp · work`, `aws · prod-admin`).
- `:ctx` still lists gcloud configurations when the AWS CLI is missing, and
  AWS profiles when gcloud is.

## [1.3.0] - 2026-11-04

### Added
- **Google Cloud folders as the tree.** Under Google Cloud, `:mg` shows the
  organization, its folders and their projects (Cloud Resource Manager).
  `Enter` on a folder lists the resources of every project below it; on a
  project, its resources. Without access to the organization (or with
  several), a root above them holds what the login can see, and folders or
  projects whose parent is hidden sit directly under it.
- **Console links for Google Cloud.** `o` opens the Google Cloud console:
  a project's dashboard, a folder or organization in Resource Manager, and
  common resources on their own page (VM instances, buckets, GKE clusters,
  Cloud SQL, Cloud Run, Cloud Functions, Pub/Sub topics, BigQuery datasets).
  Other resources open the project's Asset Inventory searching for their
  name. `y` on a project or folder copies its resource name
  (`projects/…`, `folders/…`).

### Changed
- The tree view speaks the context's words: "Folders", "N projects",
  "Enter opens resources" under Google Cloud.
- Without a browser, `o` copies the link for any cloud ("link copied").

## [1.2.0] - 2026-11-04

### Added
- **Google Cloud browsing (preview).** `:ctx` now lists gcloud
  configurations next to Azure tenants, with a cloud column; `Enter` or
  `:ctx <configuration>` switches to Google Cloud. Projects come from Cloud
  Resource Manager; `Enter` on a project lists its resources from Cloud
  Asset Inventory (type, location, labels; `/`, `type=`, `tag=` filters
  work), and `Enter` on a resource shows its document. Needs the Google
  Cloud CLI (`gcloud auth login`) and the Cloud Asset API enabled on the
  projects. `:ctx all` goes back to Azure.
- Under Google Cloud, `:mg` and `:rg` say they aren't available there yet,
  like the Azure-only views; `o` says console links are Azure only for now.

### Changed
- `:ctx` is titled "Contexts" (Azure tenants and gcloud configurations).

## [1.1.0] - 2026-11-04

### Added
- **The status bar starts with the context's cloud** (`azure`, and the
  tenant's name after `:ctx`), first step towards GCP and AWS (ROADMAP,
  M6). List titles, the breadcrumb and the status bar take their words
  from the cloud (subscription, resource group, management groups for
  Azure; projects, locations, folders for GCP; accounts, regions,
  organizational units for AWS).
- Under a context that isn't Azure, Azure-only commands (`:policy`,
  `:compliance`, `:exemption`, `:ra`, `:role`, `:access`, `:group`,
  `:app`) say so instead of failing.

### Changed
- The status bar labels the selected subscription `sub:` (was `Sub:`).

## [1.0.1] - 2026-11-04

### Fixed
- Links (README, release notes, package metadata, version comparisons) point
  to the renamed repository, github.com/mrWiecek/c9s. The old az9s URLs
  still redirect.

## [1.0.0] - 2026-11-04

### Changed
- **az9s is now c9s**, *canines for clouds*: the command, crates, release
  archives and docs carry the new name, ahead of browsing other clouds
  (see ROADMAP, M6). This is the only change, but the command's name is,
  so the version is 1.0.0.
- On first start, the az9s config file and cache move to c9s's own
  directories (e.g. `~/.config/az9s` → `~/.config/c9s`), so settings and
  cached lists carry over.
- Environment variables are now `C9S_CONFIG` and `C9S_BACKEND`; the
  `AZ9S_` names still work. `alias az9s=c9s` keeps the old command.

## [0.22.1] - 2026-11-04

### Fixed
- **Role assignments showed role IDs instead of names** when Resource Graph
  didn't list the built-in roles. Their names now come from ARM (every
  role usable at the tenant root, or at a subscription without root
  access), and `Enter` on such a role opens its definition through ARM.
- **Role assignment columns cut off the scope.** Columns are now as wide
  as their contents, and the scope comes last with the rest of the line,
  so it shows whole.
- **`p` was hard to find.** It shows everything the row's user, group or
  app can do across the tenant; it now leads the title and the key bar.
- **`:app` reshuffled apps of the same name** (e.g. managed identities)
  on every redraw. They are now ordered by name, then app ID.

## [0.22.0] - 2026-11-04

### Added
- **Eight more themes**: `high-contrast` (bright terminal colours only) and
  colour schemes with their own background, so they look the same on any
  terminal: `dracula`, `nord`, `gruvbox`, `catppuccin` (Mocha),
  `tokyo-night`, `solarized-dark` and `solarized-light`. Set one with
  `theme = "…"` in the config file. The schemes use 24-bit colour.
- **`:theme <name>`** switches the theme while az9s runs (`:theme` alone
  lists them, with the current one; Tab completes names). It lasts until
  you quit; the config file keeps it.

## [0.21.0] - 2026-11-04

### Added
- **`:app [name]`: applications** (aliases `:application`,
  `:enterpriseapp`). One row per app: an app registration and its
  enterprise application are joined by app ID, so the two are one item.
  Apps registered elsewhere (external) and managed identities are listed
  too; Microsoft's own apps are hidden until `m`. Each row shows the
  soonest credential to expire (red: expired, yellow: within 30 days); the
  title counts apps by kind and those with credentials expired or
  expiring.
- **An application's tabs** (`Enter`; `1`-`7`, `Tab`):
  - **Overview**: both object IDs, sign-in audience, created, enabled,
    assignment required, publisher tenant, redirect URIs.
  - **Permissions**: what the registration asks for next to what is
    granted to the enterprise app, delegated and application, with
    readable names (e.g. Microsoft Graph · User.Read.All) and consent:
    not granted first (red), "granted by N users", or granted but no
    longer requested (yellow).
  - **Exposed API**: identifier URIs, scopes (who can consent) and app
    roles (who can be given them).
  - **Owners** of the registration and of the enterprise app.
  - **Credentials**: every secret and certificate, soonest to expire first.
  - **Assigned**: users, groups and apps assigned, with their role (and
    whether assignment is required).
  - **JSON**: both documents.
- `p` shows the app's Azure access, `o` opens its Entra page, `y` copies
  its app ID, `c` copies the list or the current tab.

## [0.20.0] - 2026-11-04

### Added
- **`:group [name]`: directory groups** from Microsoft Graph, by name: kind
  (security, Microsoft 365, distribution), dynamic or assigned membership,
  role-assignable (`R`), mail and description; the title counts dynamic
  and role-assignable groups. `/` also matches "dynamic" and
  "role-assignable". Cached per tenant.
- **Group members**: `Enter` on a group lists its members (groups first,
  then users, apps and devices, with sign-in name or app ID), counted by
  kind in the title. `t` switches between direct and nested members.
  `Enter` on a member group opens its members; on a user or app, the
  groups it belongs to (nested too). `Esc` goes back the same way.
- `p` in these views shows the group's or member's Azure access (as in
  `:ra`); `o` opens its Entra page in the Azure Portal; `y` copies its
  object ID; `c` copies the list.

## [0.19.0] - 2026-11-03

### Added
- **One principal's access across the tenant.** `p` on a row of `:ra`, or
  `:access <name | sign-in name | app ID | object ID>`, lists every role a
  user, group or app holds: its own assignments ("direct") and those of
  every group it belongs to, nested groups included ("via *group*"). The
  title says how many of each and how many groups it is a member of.
  Groups come from Microsoft Graph; without access a notice says so and
  the direct assignments still show. `Enter`, `d` and `p` work as in `:ra`
  (`p` on a group row shows the group's own access).

### Fixed
- Going back (`Esc`) to a role assignment list restores the scope, role
  and principal it showed, instead of keeping the filter of the list you
  left.

## [0.18.0] - 2026-11-03

### Added
- **`:ra [all|<mg>|<sub>]`: role assignments** (aliases `:roleassignment`,
  `:role-assignment`) that matter where you are: inherited from the tenant
  root and management groups above (top first), made at the scope, and
  below it (resource groups, resources), marked in the title ("2 inherited,
  1 here, 1 below"). Each row shows the role (`C` custom, `B` built-in),
  the principal's name with sign-in name or app ID, its type (user, group,
  service principal) and where it is assigned. `Enter` opens the role's
  permissions; `d` the assignment; `/` finds roles, principals, types and
  scopes (e.g. `/custom`).
- **`:role`: role definitions** (aliases `:roledefinition`,
  `:role-definition`), custom first, with how many assignments each has
  ("unused" for none) and the counts in the title. `Enter` lists everyone
  with the role across the tenant; `d` opens its permissions: actions,
  not-actions, data actions and assignable scopes, one per line.
- **Principal names from Microsoft Graph**, using the same Azure CLI login.
  Looked up for the IDs not yet known, cached per tenant. Without directory
  read access (e.g. as a guest) a notice says so and IDs are shown.
- Without access at the tenant root, role assignments and definitions come
  from your subscriptions instead, with a notice (management group
  assignments may be missing).

### Fixed
- Going back (`Esc`) to `:policy`, the non-compliant list, `:compliance` or
  `:exemption` now keeps the row you were on instead of jumping to the top.

## [0.17.0] - 2026-11-03

### Added
- **`:exemption [all|<mg>|<sub>]`**: policy exemptions at or below the
  scope (a management group or subscription by name, or where you are),
  with their category (waiver or mitigated), expiry, the assignment they
  exempt from, where they are made and why. Expired ones come first in
  red, then those expiring within 30 days in yellow, then the rest; the
  title counts both. `Enter` opens an exemption's details; `/`, `c`, `y`
  and `o` work as in other lists.
- **Exempt policies name their exemption** in a resource's Policy tab
  ("exemption *name*, expires in 12 days"), and `Enter` opens it. Expired
  exemptions no longer exempt, so they are never the one named; the most
  specific active one is.

## [0.16.0] - 2026-11-03

### Added
- **Compliance in the management group tree.** Every management group and
  subscription shows its compliance: the share of evaluated resources that
  comply with every policy and how many don't (e.g. "92.4% · 140
  non-compliant"), red below 80%, green when all comply. Groups add up
  their subscriptions. One Resource Graph query for the whole tenant,
  loaded in the background with the tree and cached.
- **`:compliance [all|<mg>]`** ranks subscriptions by non-compliant
  resources, worst first, with their management group; those without
  evaluated resources come last. `Enter` opens `:policy` for the
  subscription; `/`, `c`, `y` and `o` work as in other lists.
- The `:policy` summary line shows at once from that data while the
  scope's own numbers load.

## [0.15.0] - 2026-11-03

### Added
- **Open a tag from a resource.** In the detail view's Tags tab, `j`/`k`
  select a tag and `Enter` lists every resource in the tenant with that
  tag (`tag=key=value` across all subscriptions). `Esc` goes back to the
  resource, then to the list you came from, as it was.
- **Policy tab: where each policy comes from, and its definition.** Each
  row now shows the policy's name, whether it came with an initiative
  (and which) or was assigned directly, the assignment, and where it is
  assigned (management group, subscription or resource group). `Enter`
  opens the policy definition: type, category, version, mode, effect (with
  its allowed values), description, and the full JSON. Built-in
  definitions that Resource Graph doesn't list are read from ARM.
- `c` in the Policy tab copies the new columns, with definition and
  assignment IDs.

## [0.14.0] - 2026-11-03

### Added
- **Compliance summary in `:policy`.** The first line shows how compliant
  the scope is: the share of evaluated resources that comply with every
  policy, how many resources are non-compliant (each counted once, however
  many policies they fail), how many subscriptions are affected (for a
  management group) and how many assignments have non-compliant resources.
  Cached like the rest of the view, so it shows at once and refreshes in
  the background.

## [0.13.0] - 2026-11-03

### Added
- **Policy drill-down.** In `:policy`, `Enter` on an assignment lists the
  resources that are non-compliant with it in that scope, one row per
  resource with the failing policies (for an initiative: "+N more"). `/`
  filters by name, type, resource group or policy; `c` copies the list.
  `Enter` on a resource opens its detail view. `d` opens the assignment
  itself (what `Enter` did before).
- **Policy tab in the detail view** (`3`; JSON moved to `4`): every policy
  that evaluates the resource, with its state, non-compliant first. Loaded
  when you open the tab, for any resource.

## [0.12.1] - 2026-11-03

### Fixed
- **On Windows, az9s could not find the Azure CLI** ("program not found"),
  so the subscription list never loaded, although `az` worked in the same
  PowerShell window. On Windows `az` is a batch file (`az.cmd`), which
  programs can't start by bare name; az9s now runs it through `cmd /C`, as
  the Azure SDK does for tokens. This also affected `:ctx` tenant tokens.
  If the CLI really is missing, the error now says so plainly.
- **Without read access to management groups**, the status bar showed
  Azure's long "forbidden" message, cut off. It now says "No read access to
  management groups: showing subscriptions", and which commands still work.
  az9s starts on the subscription list as before.

## [0.12.0] - 2026-11-03

### Added
- **Azure Policy, first step: `:policy [all|<sub>]`.** Lists the policy
  assignments that apply at a scope: assigned there ("this scope") or
  inherited from the management groups above it ("from Platform"). Each row
  shows whether it is an initiative (I) or a single policy (P), whether it
  is enforced, and how many resources are non-compliant out of those
  evaluated, worst first. The title says how many more assignments sit below
  the scope.
  - `:policy` uses where you are (the highlighted or current subscription or
    management group); `:policy all` the whole tenant (needs the management
    group tree).
  - `Enter` opens the assignment in the detail view (Overview, JSON, `/`
    search); `c`, `y`, `o` and `/` work as in other lists.
  - Data: two Resource Graph queries on `policyresources` (all assignments
    tenant-wide, and compliance counts for the scope from the latest policy
    states), cached per tenant and per scope so the view opens instantly
    next time.
- The detail view finds policy objects (in `policyresources`, possibly on a
  management group) as well as resources.

## [0.11.0] - 2026-11-03

### Added
- **Themes:** `theme = "default" | "light" | "mono"` in the config file.
  - `default`: the colours so far, for dark terminals.
  - `light`: for light terminal backgrounds: no white, grey or yellow text;
    blue accents and selection.
  - `mono`: no colours at all; the selected row and search matches use
    reversed and underlined text. The `NO_COLOR` environment variable
    (https://no-color.org) also selects it.

### Changed
- Every view takes its colours from one palette of roles (accent, muted,
  highlight, …) instead of fixed colours, so themes apply everywhere.

## [0.10.0] - 2026-11-03

### Added
- **`:ctx`: switch tenant.** `:ctx` lists the tenants you can use, with
  names and domains from Azure, how many of your subscriptions each has,
  and ● on the current one; `Enter` switches. `:ctx <name|domain|ID>`
  switches directly, `:ctx all` returns to the Azure CLI's default login.
  - After a switch, Azure is asked as that tenant (tokens from `az account
    get-access-token --tenant`), and subscriptions, `all`, the management
    group tree (cached per tenant), the palette and suggestions cover that
    tenant only. The breadcrumb shows `[tenant]`.
  - If the CLI has no token for the tenant, the error says to run
    `az login --tenant <id>` once.
  - `tenant = "<id>"` in the config file starts in that tenant.
  - The ARM fallback (`AZ9S_BACKEND=arm`) keeps the default login.

## [0.9.0] - 2026-11-03

### Added
- **Config file**, optional: `config.toml` in the config directory
  (`~/.config/az9s/` on Linux and WSL; `az9s --help` shows the path), or
  `--config <path>` / `AZ9S_CONFIG`. `az9s --init-config` writes a
  commented one with the defaults (it never overwrites). Settings:
  - `start = "mg" | "sub"`: the first view.
  - `default_subscription`: what `:rg`, `:res`, `:type` and `:tag` use when
    nothing is highlighted or open (otherwise all subscriptions).
  - `refresh_interval`: reload the current view every N seconds (at least
    30; 0, the default, is off). It waits while you type, search, use the
    palette or read help. The status bar shows `auto ⟳ Ns`.
  - `read_only` (default `true`): the status bar shows `RO`; actions that
    change Azure (M4) will check it.

  A broken file never stops az9s: it starts with the defaults and the
  status bar says which line is wrong and why (e.g. `line 2: unknown field
  refresh_intervall, expected one of …`).

## [0.8.0] - 2026-11-03

### Added
- **Command palette: `Ctrl+P`.** One fuzzy search over commands,
  subscriptions, management groups, and the loaded resource groups and
  resources. Type a few letters (`kvprod` finds `kv-prod-weu`; several
  words must all match), `↑`/`↓` choose, `Enter` goes there: a command
  runs, a subscription or management group opens its resource groups, a
  resource group its resources, a resource its detail view. `Esc` from
  there returns to where the palette was opened. Matched letters are
  highlighted. It searches what is loaded, so after `:res all` it finds any
  resource (100k resources: about 30 ms to open, 25 ms per keystroke).
- **Autocomplete for `:` commands.** Suggestions appear above the command
  line as you type: command names, then `all` and subscription names, then
  filter keys, then values from what is loaded: aliases after `type=`,
  locations after `loc=`, tag keys after `tag=` and that key's values after
  `tag=key=`. `Tab` completes, `↑`/`↓` choose.

## [0.7.0] - 2026-11-03

### Added
- **Tags view: `:tag [all|<sub>] [filters]`.** Lists every tag on the
  resources in scope as key, value and how many resources carry it, most
  common first. Keys are grouped ignoring case, as Azure treats them. `/`
  narrows by key or value, `Enter` opens the resources with that tag
  (without reloading), `Esc` comes back, `c` copies the list. Filters work
  as for `:res`, e.g. `:tag all type=keyvault`.
- **Vim-style search in the resource detail view.** `/` searches the open
  tab (Overview, Tags or JSON) instead of filtering: it jumps to the first
  match as you type and highlights every match, the current one more
  strongly. `Enter` keeps the search, `Esc` cancels it and returns to where
  you were. `n` / `N` go to the next / previous match and wrap around
  ("search hit BOTTOM, continuing at TOP"); the status bar shows "match 3
  of 12". Lowercase ignores case, a capital letter makes it exact
  (smartcase). `Esc` clears the highlights, then goes back.

## [0.6.0] - 2026-11-02

### Added
- **`c` copies the current view**, in every view:
  - lists (subscriptions, resource groups, resources): every row that passes
    the filters, not only the rows on screen, tab-separated with a header,
    so it pastes into a spreadsheet;
  - the resource detail view: the current tab (Overview, Tags or JSON);
  - the management group tree and the `:type` summary: their rows.
  The status bar says what was copied and how much.
- **VNet subnets in the detail view.** A virtual network's Overview lists
  each subnet with its prefixes, delegations (e.g. `delegated to
  Microsoft.Web/serverFarms`, or `no delegation`), NSG, route table and NAT
  gateway.
- **`:rg` takes filters too:** `loc=` and `tag=`, e.g. `:rg all
  loc=northeurope` or `:rg tag=owner`. `type=` there is an error that
  points to `:res` and `:type`. Active filters show in every list's title.
- **Type aliases in filters.** `type=vnet`, `type=vm`, `type=key-vault`
  (or `keyvault`) work, as does a provider without `Microsoft.`
  (`type=network`). The `/` filter in resource lists and in `:type` matches
  aliases too (typing `vnet` in `:type` finds
  `microsoft.network/virtualnetworks`). Before, `type=` needed the type's
  full name or its last part.
- **`tag=environment=prod`** works as well as `tag=environment:prod`.
- **A rewritten, scrollable help (`?`)**: commands, the scope rules,
  every filter with its matching rules, worked examples, and all keys.
  `j`/`k`, `g`/`G` and `Ctrl+D`/`Ctrl+U` scroll it.

### Changed
- List columns no longer overlap: long names are cut with "…" leaving a
  gap before the next column, and the name column widens with the terminal.

### Fixed
- **`o` failed on WSL** with an OS error and a cut-off link: az9s ran
  `xdg-open`, which WSL usually doesn't have. On WSL it now opens the
  Windows browser (`wslview`, then `cmd.exe /c start`, then `xdg-open`).
  If no browser can be started anywhere, the Portal link is copied to the
  clipboard instead, with a short message, rather than printed into the
  status bar where it was truncated.
- **`:rg`, `:res` and `:type` without a scope always opened all
  subscriptions.** Without a scope they now apply to where you are: the
  highlighted subscription (subscription list, tree), the highlighted
  management group (tree), or the subscription or management group the
  current list is scoped to. Only with no such context, or with `all`, do
  they cover all subscriptions.

## [0.5.0] - 2026-11-02

### Added
- **Resource filters:** `:res [all|<sub>] type=… loc=… tag=…`.
  - `type=` matches the full type (`Microsoft.Compute/virtualMachines`), its
    last part (`virtualMachines`) or a provider (`Microsoft.Compute`).
  - `loc=` matches a location; `tag=env:prod` a tag value, `tag=env` any
    resource with that tag.
  - Different filters must all match; comma-separated values are
    alternatives (`type=virtualMachines,disks`). Case doesn't matter.
  - Filters apply to the loaded list on your machine, so they are instant
    (cached rows included), combine with `/`, show in the list title, and
    survive `r` and going back. A new `:res` without filters clears them.
  - An unknown filter (e.g. `size=big`) is an error that names the valid
    ones.
- **Resource type summary:** `:type [all|<sub>] [filters]` lists how many
  resources of each type there are, most common first. `Enter` on a type
  opens the resource list filtered to it, without reloading; `Esc` returns.
- The `perf` CI job also checks filtered lists and the type summary at 100k
  resources (each frame under 1 ms on CI hardware today; the budget is 16 ms).

## [0.4.0] - 2026-11-02

### Added
- **Resource detail view with Overview, Tags and JSON tabs.** `Enter` on a
  resource opens it at once with what the list knows, then loads the full
  resource document (one Resource Graph query) in the background.
  - Overview: name, type, kind, location, zones, resource group,
    subscription, provisioning state, managed by, ID, plus SKU, managed
    identity, plan, and the values in `properties` as dotted names (e.g.
    `hardwareProfile.vmSize`, up to 60 rows).
  - Tags: sorted by key. JSON: the whole document, colour-highlighted.
  - `Tab`/`Shift+Tab` (or `l`/`h`, `1`-`3`) switch tabs; `j`/`k`, `g`/`G`,
    `Ctrl+D`/`Ctrl+U` scroll; `r` reloads.
  - Documents are cached: reopening a resource shows it instantly, then
    refreshes. If loading fails, the basic view stays with a status-bar
    notice instead of an error popup.
- **`y` copies the selected item's Azure ID** (subscription, management
  group, resource group or resource) in every view. It uses the terminal's
  clipboard support (OSC 52), so it also works over SSH; in tmux, enable
  `set-clipboard`.
- **`o` opens the selected item in the Azure Portal**, in the right tenant.
  If no browser can be started, the status bar shows the link.
- `azure.resource_detail` in `--timings`.

### Changed
- Refreshing a resource list now deletes only the cached resources that are
  gone from Azure, instead of rewriting the whole scope, so cached resource
  details survive a refresh.

## [0.3.1] - 2026-11-02

### Fixed
- **Starting two az9s processes at once could crash with "UNIQUE constraint
  failed: schema_migrations.version"** (#9), e.g. right after upgrading to
  0.3.0, whose cache format change (migration v2) both tried to apply.
  Migrations now run in one transaction that takes the database's write
  lock first; the other process waits and finds nothing left to do.
- Two processes creating a new cache file at the same moment could fail with
  "database is locked" while switching it to WAL mode. az9s now retries.

## [0.3.0] - 2026-11-02

### Added
- **Management group tree, the new start view** (`:mg`). az9s opens at the
  tenant root management group and shows the hierarchy down to subscriptions,
  with the number of subscriptions below each group.
  - `Space` opens or closes a group; `l`/`→` opens, `h`/`←` closes (on a
    closed row it jumps to the parent).
  - `Enter` on a group lists the resource groups of every enabled
    subscription below it, in one Resource Graph query. `r` refreshes that
    same scope, and `Esc` returns to the tree with the group still selected.
    `Enter` on a subscription opens its resource groups.
  - `/` filters the tree; matches are shown with their parent groups opened.
  - Subscriptions you can see but that are not in the tree (e.g. from another
    tenant) are listed under "Not in the management group tree".
  - The tree is cached and shown instantly on the next start, then refreshed
    in the background (one call to the management group descendants API).
  - If the tree can't be loaded (no read access on the tenant root, or
    `AZ9S_BACKEND=arm`) and nothing is cached, az9s starts on the
    subscription list and says why in the status bar.
- Going back (`Esc`) now restores the selected row in every list, not just the
  first row.
- `azure.management_groups` in `--timings`.

### Fixed
- A long subscription name with non-ASCII characters could crash the header
  breadcrumb (byte-index string slicing).

## [0.2.2] - 2026-11-02

### Fixed
- The declared minimum Rust version was 1.75, but dependencies require
  **1.88** (`time 0.3.47`). `rust-version` and the README now say 1.88, and a
  new `msrv` CI job builds with exactly that version.

### Changed
- CI runs the 100k-row rendering benchmark (`perf` job) and fails if a frame
  takes 16 ms or more.

## [0.2.1] - 2026-11-02

### Changed
- **Faster complete loads for large scopes** (#9). Resource Graph results come
  in 1000-row pages, previously fetched one after another (about 0.6 s each).
  After the first page reports the total, the remaining pages are now fetched
  4 at a time by offset and still delivered in order. If offset paging fails,
  az9s continues page by page without repeating rows.
- **The Azure token is fetched once and reused** until shortly before it
  expires, instead of once per page. Concurrent requests share one fetch.
- **The token and tenant are fetched in the background at startup**, so the
  first "all subscriptions" view doesn't wait about 0.8 s for the Azure CLI.
- New `azure.token`, `azure.graph_page` and `azure.warm_up` entries in
  `--timings`.

### Fixed
- The tenant used for "all subscriptions" queries was looked up again on every
  request. Its cache was cloned per request (a cloned `OnceCell` is a copy), so
  each `:rg all` / `:res all` paid an extra Azure CLI token call.

## [0.2.0] - 2026-11-02

### Added
- **Cache-first views.** Opening a resource group or resource list (one
  subscription, one resource group, or `:rg all` / `:res all`) shows the cached
  rows for exactly that scope immediately, then refreshes from Azure in the
  background.
  - When cached rows are on screen, fresh pages are collected off-screen and
    swapped in once, so the list doesn't shrink to the first page. The
    selection stays on the same item.
  - With nothing cached, pages stream in as before.
- **Data age in the status bar** for the current view, e.g. `● Azure · just now`
  or `◐ Cached · 3h ago`.

### Changed
- When Azure can't be reached but cached rows are on screen, az9s shows a
  status-bar warning instead of an error popup.
- Cache writes run on a background thread, and the database uses WAL mode.
  Writing 20k resources takes about 0.5 s, which used to block the UI.
- A completed refresh replaces the cached scope as a whole, so resources and
  resource groups deleted in Azure disappear from the cache. Subscriptions that
  failed to load keep their cached rows.

### Fixed
- **Every start of az9s wiped the cached resource groups and resources.**
  Refreshing subscriptions used `INSERT OR REPLACE`, which SQLite runs as
  delete + insert, and the schema's `ON DELETE CASCADE` then deleted every
  child row. All cache writes to parent tables now use upserts.
- A long subscription name with non-ASCII characters could crash the status
  bar (byte-index string slicing).

## [0.1.0] - 2026-11-02

First tagged release.

### Added
- **Versioning and releases.** Plain SemVer: patch for fixes, minor for features,
  major for breaking changes. Merging a version bump to `main` tags it and
  publishes a GitHub Release with binaries for Linux, macOS (Apple Silicon and
  Intel) and Windows, SHA-256 checksums, and that version's notes from this file.
  `just bump patch|minor|major` prepares a release, and CI fails a PR that
  changes code without bumping an already-released version. See `RELEASING.md`
  for auditing and rolling back.
- `az9s --version` shows the git commit the binary was built from.
- `ROADMAP.md` with milestones M0–M5 and recorded product decisions.
- GitHub Actions CI: `fmt`, `clippy -D warnings`, and tests on Linux, macOS and
  Windows.
- `just ci`, `just install` and `just run-debug` (logs to `az9s.log`).
- `CONTRIBUTING.md`, `docs/ARCHITECTURE.md` (describes the current code), PR and
  issue templates, `CODEOWNERS`, `rustfmt.toml`.

### Changed
- `az9s --help` and `--version` are now available; unknown arguments are rejected.
- **Renamed to az9s.** The binary is now `az9s` (was `aztui`). Crates are `az9s`,
  `az9s-core`, `az9s-azure` and `az9s-tui`. The local data dir is now `az9s`. The old
  `aztui` cache is not migrated; it is only a cache and is rebuilt on first run.
- **Flattened the repository.** The Cargo workspace moved from `src/apps/aztui/` to
  the repo root.
- **License is now Apache-2.0 only** (was MIT in `LICENSE`, "MIT OR Apache-2.0" in
  `Cargo.toml`).
- The cache location comes from the `directories` crate, so it is correct on Windows
  and macOS too.
- Logging is off unless `RUST_LOG` is set, and goes to stderr so it no longer
  corrupts the TUI.
- README rewritten for platform engineers.
- Ran `cargo fmt` across the codebase.
- **Commands are now singular:** `:sub` (was `:subs`), `:rg` (was `:rgs`),
  `:res` / `:resource` (was `:resources`). Plural forms are no longer accepted.

### Performance (M1, step 1)
- **Input is no longer rate-limited.** Previously at most one key was handled per
  250 ms tick. Now every key is handled as it arrives, and queued keys are processed
  together before one redraw.
- **List rendering is virtualized.** Only the visible rows are built each frame,
  filtering no longer allocates, and filter results are memoized. With 100k
  resources, a frame dropped from 78 ms to 0.4 ms (101 ms to 0.6 ms with an active
  filter).
- New `--timings` flag prints Azure, cache, frame and keypress timings on exit.
  New `just run-timings` and `just perf` recipes.

### Performance (M1, step 2)
- **Azure calls run in the background; the UI never freezes.** Opening a view
  switches to it immediately, with an animated spinner in the status bar, and the
  list fills in when Azure answers. You can keep navigating, go back, or open
  something else. A superseded request is cancelled and its result discarded.
- **Startup no longer waits for Azure.** Cached subscriptions are shown at once and
  refreshed in the background.
- **"All subscriptions" views (`:rg all`, `:res all`) load 16 subscriptions at a
  time** instead of one after another, and skip disabled subscriptions. If some
  subscriptions fail, the rest still show, with a warning in the status bar.

### Performance (M1, step 3)
- **Resource Graph backend.** Resource group and resource lists now come from Azure
  Resource Graph instead of one ARM call per subscription.
  - `:rg all` and `:res all` run one query at the tenant root management group,
    limited to your enabled subscriptions. If you can't read the root, az9s
    retries scoped to your subscriptions.
  - Results stream in page by page (1000 rows per page), so the first rows show
    before the whole list has loaded.
  - Opening a resource group runs a filtered query in that subscription.
  - Resource Graph throttling is respected: az9s waits out the quota and retries
    429s.
  - If Resource Graph fails before returning any rows, az9s falls back to ARM and
    says so in the status bar. `AZ9S_BACKEND=arm` forces the old ARM path.
  - Trade-off: Resource Graph can lag behind ARM by a short time after changes.

### Fixed
- **Installing no longer requires system OpenSSL or `pkg-config`** (#4). TLS now
  uses rustls, with the bundled Mozilla roots plus the OS certificate store, so
  corporate TLS-inspecting proxies keep working. The unused `azure_mgmt_compute`
  and `azure_mgmt_network` dependencies were removed. CI fails if OpenSSL
  reappears in the dependency tree.
- Resource types from Resource Graph come in lowercase. Short type labels and
  colours are now matched case-insensitively.
- Opening a resource group showed every resource in its subscription. It now
  requests only that group's resources from Azure.
- In `:rg all`, opening a resource group used the wrong subscription. It now uses
  the group's own subscription.
- Going back to an "all subscriptions" list lost its scope, so `r` refreshed
  nothing.
- Scrolling up in a long list no longer pins the selection to the bottom row. The
  scroll offset is now kept between frames.
- A resource name longer than 35 characters containing non-ASCII characters could
  crash the app (byte-index string slicing).
- On Windows, key release events are ignored so each key press acts once.

### Removed
- `src/apps/aztui/docs/`: about 80 archived iteration reports and fix scripts, plus
  about 8k lines of outdated design docs. They remain in git history.
- The non-functional `Ctrl+P` "command palette" toggle. A real palette is planned for
  M2.
- An unused Azure SDK client that was built on every subscription fetch.
- The old `src/apps/aztui/CHANGELOG.md`, which described an unreleased "0.1.0" with
  unverified claims.

### Existing functionality (development baseline)
Already in the codebase before this changelog was started:
- Azure CLI authentication, falling back to the default credential chain.
- Browsing subscriptions → resource groups → resources → a basic resource detail view.
- Cross-subscription views (now `:rg all` and `:res all`).
- Vim-style navigation (`j/k`, `g/G`, `Ctrl+D/U`), `/` filtering, `:` commands, and
  `?` help.
- A SQLite cache, used as a fallback when Azure is unreachable at startup.
- A Rust workspace with 4 crates and 65 passing tests.

### Known issues
- The Resource Graph backend is tested against a mock API but not yet against a
  live 400+ subscription tenant.
- Resource groups and resources are not yet shown from the cache before Azure
  answers (M1 step 4).

[Unreleased]: https://github.com/mrWiecek/c9s/compare/v1.9.1...HEAD
[1.9.1]: https://github.com/mrWiecek/c9s/compare/v1.9.0...v1.9.1
[1.9.0]: https://github.com/mrWiecek/c9s/compare/v1.8.0...v1.9.0
[1.8.0]: https://github.com/mrWiecek/c9s/compare/v1.7.0...v1.8.0
[1.7.0]: https://github.com/mrWiecek/c9s/compare/v1.6.1...v1.7.0
[1.6.1]: https://github.com/mrWiecek/c9s/compare/v1.6.0...v1.6.1
[1.6.0]: https://github.com/mrWiecek/c9s/compare/v1.5.2...v1.6.0
[1.5.2]: https://github.com/mrWiecek/c9s/compare/v1.5.1...v1.5.2
[1.5.1]: https://github.com/mrWiecek/c9s/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/mrWiecek/c9s/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/mrWiecek/c9s/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/mrWiecek/c9s/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/mrWiecek/c9s/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/mrWiecek/c9s/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/mrWiecek/c9s/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/mrWiecek/c9s/compare/v0.22.1...v1.0.0
[0.22.1]: https://github.com/mrWiecek/c9s/compare/v0.22.0...v0.22.1
[0.22.0]: https://github.com/mrWiecek/c9s/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/mrWiecek/c9s/compare/v0.20.0...v0.21.0
[0.20.0]: https://github.com/mrWiecek/c9s/compare/v0.19.0...v0.20.0
[0.19.0]: https://github.com/mrWiecek/c9s/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/mrWiecek/c9s/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/mrWiecek/c9s/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/mrWiecek/c9s/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/mrWiecek/c9s/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/mrWiecek/c9s/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/mrWiecek/c9s/compare/v0.12.1...v0.13.0
[0.12.1]: https://github.com/mrWiecek/c9s/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/mrWiecek/c9s/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/mrWiecek/c9s/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/mrWiecek/c9s/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/mrWiecek/c9s/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/mrWiecek/c9s/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/mrWiecek/c9s/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/mrWiecek/c9s/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/mrWiecek/c9s/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/mrWiecek/c9s/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/mrWiecek/c9s/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/mrWiecek/c9s/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/mrWiecek/c9s/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/mrWiecek/c9s/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/mrWiecek/c9s/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/mrWiecek/c9s/releases/tag/v0.1.0
