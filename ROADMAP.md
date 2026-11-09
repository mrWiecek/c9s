# c9s Roadmap

c9s (*canines for clouds*; az9s before v1.0.0) is a fast, keyboard-first terminal tool for **platform engineers** who operate Azure,
in the spirit of k9s: browse everything, find things instantly, act on them safely, and
see policy compliance without leaving the terminal.

> Status: **pre-release / development**. Nothing has been published yet.
> Milestones are sized in weeks for a single maintainer and are estimates, not promises.

## Guiding principles

1. **Speed is the feature.** First frame in under 200 ms (from cache), every keypress
   renders in under 16 ms, and the UI never blocks on the network.
2. **Breadth first, depth where it matters.** Every Azure resource gets a useful generic
   view (properties, tags, raw JSON). A small set of resource types that platform
   engineers touch daily get rich views and actions.
3. **Safe by default.** Actions are enabled, but every mutating action needs
   confirmation. Destructive actions need you to type the resource name. A
   `--readonly` flag or config setting disables all actions.
4. **Bring your own login.** Authentication goes through the Azure CLI (`az login`). c9s
   stores no credentials.
5. **Open source, cross-platform.** It must work on Linux, macOS and Windows, and CI
   proves that. Licensed under Apache-2.0.
6. **Commands are singular.** `:sub`, `:rg`, `:res`, `:mg`, `:policy`,
   `:assignment`, `:exemption`. Plural forms are not accepted.
7. **Management groups are first-class.** Platform engineers work at management-group
   scope. The scope tree (tenant root → management groups → subscriptions) is the
   top of navigation, and Policy is viewed per scope.

## Milestones at a glance

Versions are not tied to milestones. Every merged change ships as its own
SemVer release: patch for fixes, minor for features, major for breaking
changes (see [RELEASING.md](RELEASING.md)). The first tagged release is
**v0.1.0**.

| # | Milestone | Size | Outcome |
|---|---|---|---|
| M0 | Foundation & cleanup | 1–2 wk | One name, flat repo, honest docs, CI green |
| M1 | Speed | 2–3 wk | Instant UI, Resource Graph backend |
| M2 | Inspect | 2 wk | Real detail views, palette, config |
| M3 | Policy | 3 wk | Azure Policy at management-group scope: assignments, compliance, exemptions |
| M4 | Operate | 3 wk | Safe action framework + first actions |
| M5 | Rich views & polish | ongoing | AKS, Key Vault, logs, themes, packaging |
| M6 | Multi-cloud: c9s | ~13–19 days | Rename to **c9s** ("canines for clouds"); browse GCP and AWS next to Azure |

---

## M0 — Foundation & cleanup (1–2 weeks)

Goal: a repository a stranger can clone, understand and contribute to in 10 minutes.

- [x] **One name: az9s.** Rename the binary (`aztui` → `az9s`), crates (`az9s`,
      `az9s-core`, `az9s-azure`, `az9s-tui`), cache dir, log targets, and the
      `repository` URL in `Cargo.toml`.
- [x] **Flatten the repo.** Move `src/apps/aztui/*` to the repo root
      (`Cargo.toml` + `crates/`) and update the `justfile`.
- [x] **Docs cleanup.** Deleted the old docs folder: about 80 archived iteration
      reports and fix scripts, plus about 8k lines of outdated design docs. Git
      history keeps them. Replaced with `README.md`, `CONTRIBUTING.md` and
      `docs/ARCHITECTURE.md`.
- [x] **License.** Apache-2.0 only. `LICENSE` and `Cargo.toml` now agree.
- [x] **Formatting & lint.** Run `cargo fmt --all` once and add `rustfmt.toml`.
      Keep clippy clean with `-D warnings`.
- [x] **CI (GitHub Actions).** Run fmt, clippy and test on Linux, macOS and Windows
      for every PR.
- [x] **Small correctness fixes.**
  - Replace the hand-written `dirs` stub with the `directories` crate (the stub
    breaks on Windows).
  - Remove the dead `Ctrl+P` "command palette" toggle until the palette exists.
  - Remove the unused SDK client built in `list_subscriptions`.
- [x] Add a PR template, an issue template and `CODEOWNERS`.

**Done when:** `just ci` passes locally and in GitHub Actions, the README is accurate,
and no file or crate says "aztui".

CI is green on Linux, macOS and Windows. The minimum Rust version is 1.89
(`time 0.3.47` requires it), and a CI job builds with exactly that version.

## M1 — Speed (2–3 weeks)

Goal: fix the main complaint that az9s is slow, **at the scale of the target user:
400+ subscriptions and thousands of resources** under a management-group hierarchy,
with read access at the tenant root management group. Targets below use 100k rows as
a safety margin.

Root causes found in the current code:

| Cause | Where | Effect |
|---|---|---|
| Input loop reads **one** key per 250 ms tick | `crates/az9s-tui/src/event.rs` | Navigation lags; holding `j` crawls |
| Azure calls are `await`ed inside the key handler | `crates/az9s-tui/src/app.rs` | UI freezes, spinner never draws |
| Opening a resource group refetches **every resource in the subscription** | `handle_resource_group_select` | Seconds per click on large subs |
| "All subscriptions" views loop over subscriptions **sequentially** | `load_all_*` | `:res all` = 400+ sequential ARM list calls, which takes minutes |
| Every frame re-filters (lowercasing every field) and builds a `ListItem` for **every** row, not just the visible ones | `crates/az9s-tui/src/ui/mod.rs`, `state.rs` | Each frame costs O(total rows); at 100k rows every keypress stalls |
| Startup blocks on `az account list` before the first frame | `App::new` | Slow launch |
| Cache is written but only read as a fallback | `crates/az9s-core` | Data is fetched again on every navigation |
| **Bug:** opening a resource group shows every resource in the subscription; the view is never filtered to the RG | `handle_resource_group_select` | Wrong results, and slow too |

Work items, in order:

- [x] **Measure first.** Add durations around every Azure call, cache write,
      frame render and key event, plus a `--timings` summary on exit. Record
      before and after numbers against a 400+ subscription tenant in the PR
      description.
- [x] **Event loop rewrite.** A dedicated input thread forwards every event
      immediately. The main loop drains all pending events before redrawing.
- [x] **Async architecture.** Move all I/O into background tasks that report
      results to the UI over a channel (an action/message pattern). The UI thread
      only renders and updates state. Cancel stale requests when the user moves on.
      *(Done: `az9s_core::loader`. Also, multi-subscription views now run up to 16
      ARM calls at a time as a stopgap until Resource Graph.)*
- [x] **Azure Resource Graph as the read backend.** One KQL query lists resource
      groups and resources across all subscriptions, with server-side filtering
      (by RG, type, location or tag) and paging. The same backend serves Policy
      later (`policyresources` table).
  - **Scope by management group**, not by passing 400+ subscription IDs. The default
    scope is the **tenant root management group**, so one query covers the whole
    estate. Fall back to batched subscription lists only when no management group
    is readable.
  - **Never pull the whole tenant into memory by default.** Fetch counts first
    (`summarize count()`), then page rows in (`$top` / `$skipToken`) as the user
    scrolls.
  - **Filtering:** filter locally while the full result set is in memory. Above a
    threshold, push `/` filters into the KQL `where` clause (debounced).
  - **Respect Resource Graph throttling.** Read the per-user quota headers, back
    off, and never fire one query per subscription.
  - Trade-off: Resource Graph indexes changes with a small delay. After an action,
    refresh the single resource through ARM `GET`.
- [x] **Virtualized rendering.** Build widgets only for the visible rows, persist
      the scroll offset, filter without allocating, and memoize the filter result.
      With 100k rows, a frame went from 78 ms to 0.4 ms (101 ms to 0.6 ms with a
      filter active), measured with `just perf`.
- [x] **Fix the RG view bug.** Resources shown under a resource group are filtered to
      that group on the server (ARM `list_by_resource_group` for now; Resource
      Graph in step 3).
- [x] **Cache-first (stale-while-revalidate).** Render cached data immediately,
      refresh in the background, and show data age in the status bar.
- [x] **Scope-aware queries.** Resource Graph queries can target a management group
      as well as a list of subscriptions.
      *(Validated live in #9 with 412 subscriptions and 3,469 resource groups:
      first rows in 1.59 s, complete in 3.36 s. v0.2.1 adds parallel pages and
      token reuse to bring the complete load under the 3 s target.)*
- [x] **Shared clients.** Create one HTTP client and one credential at startup and
      cache the token. *(The Resource Graph client, credential and cached token
      are shared and warmed up at startup. The ARM fallback still builds its
      client per call.)*
- [x] **Performance budget in CI.** Add benchmarks for rendering and filtering
      100k-row lists with synthetic data (no Azure needed), plus a startup-time
      check. *(The `perf` CI job runs the 100k-row rendering budget, under 16 ms
      per frame. Startup time is covered by the live test in #9 rather than CI.)*
- [x] **Release pipeline.** Merging a version bump to `main` tags it and publishes a
      GitHub Release with Linux, macOS and Windows binaries and checksums
      (`.github/workflows/release.yml`). `cargo install --git … --tag vX.Y.Z az9s`
      works too. Publishing to crates.io is not planned yet.

**Done when** (measured against a 400+ subscription tenant):

| Scenario | Target |
|---|---|
| Launch with a warm cache → first usable frame | < 200 ms |
| Keypress → rendered frame (scroll or filter), 100k rows loaded | < 16 ms |
| `:rg all` across 400+ subscriptions, cold | < 3 s |
| `:res all`, first page visible, cold | < 3 s |
| Open a resource group (≤ 1k resources), cold | < 1 s |
| Any network call | never blocks input; a spinner is always visible |

## M2 — Inspect (2 weeks)

- [x] Generic resource detail view with three tabs: Overview, Tags, JSON (from
      Resource Graph).
- [x] `y` copies the resource ID, `o` opens the resource in the Azure Portal.
- [x] Filters: `:res type=Microsoft.Compute/virtualMachines`, `loc=westeurope`,
      `tag=env:prod`, and a resource-type summary view (`:type`).
- [x] A real command palette with fuzzy search over commands and resources, and
      autocomplete for `:` commands.
- [x] Config file `~/.config/az9s/config.toml` for default subscription, refresh
      interval, start view and read-only mode (`az9s --init-config` writes one).
- [x] Themes: `theme = "default" | "light" | "mono"` in the config file; every
      view takes its colours from one palette (`ui/theme.rs`); `NO_COLOR` forces
      mono.
- [x] **Scope tree:** tenant root → management groups → subscriptions → resource
      groups, with `:mg` to jump. This becomes the top-level navigation (needed by M3),
      and az9s opens at the tenant root management group by default.
- [x] `:ctx` to switch tenant.

## M3 — Policy (3 weeks)

- [x] `:policy [all|<sub>]`: the policy assignments that apply at a scope, **per
      scope starting at management groups, with inherited assignments shown at
      child scopes** ("from Platform"), each with its compliance (non-compliant
      out of evaluated resources, from Resource Graph `policyresources`), worst
      first. `Enter` opens the assignment's details (v0.12.0).
- [ ] `:assignment` and `:exemption` commands; browse definitions and initiatives
      (set definitions).
- [x] Compliance summary per scope (totals across assignments): % of evaluated
      resources compliant, non-compliant resources (each counted once),
      subscriptions and assignments affected, at the top of `:policy` (v0.14.0).
- [x] Drill down: assignment → non-compliant resources (with the failing
      policies) → resource detail. Also the reverse: the detail view's Policy
      tab lists the resource's policy states (v0.13.0).
- [x] Compliance roll-up across the management-group tree: every group and
      subscription in the tree shows its compliance (groups add up their
      subscriptions); `:compliance [all|<mg>]` ranks subscriptions by
      non-compliant resources (v0.16.0).
- [ ] ~~The same per resource group~~: not needed, subscription level is
      enough (decided after v0.16.0).
- [x] List exemptions (with expiry), read-only: `:exemption [all|<mg>|<sub>]`,
      soonest to expire first; a resource's Exempt policies name their
      exemption (v0.17.0).
- [ ] Remediation tasks, read-only (status and resources succeeded/failed per
      assignment; not in Resource Graph, so ARM per scope). Parked: policy views
      are good enough for now.
- [x] **Access (RBAC):** `:ra [all|<mg>|<sub>]` role assignments that matter at
      a scope (inherited, here, below) with principal names from Microsoft
      Graph; `:role` role definitions, custom vs built-in, with assignment
      counts; drill-downs role ↔ assignments ↔ permissions (v0.18.0).
- [x] A principal's access across the tenant: `p` in `:ra` or `:access
      <principal>`, its own assignments and its groups' (nested too) (v0.19.0).
- [ ] Access next: PIM eligible assignments, deny assignments.
- [x] **Directory: groups.** `:group [name]` lists groups (security, Microsoft
      365, distribution; dynamic; role-assignable); members direct or nested
      (`t`); a member's groups; `p` Azure access, `o` Entra page (v0.20.0).
- [x] **Directory: applications.** `:app`: app registration and enterprise
      application as one item with tabs (overview, requested vs granted
      permissions, exposed API, owners, credentials with expiry, assigned
      users and groups, JSON) (v0.21.0).
- [ ] Directory next: owners' `Enter` to their access, users list (`:user`),
      Entra ID (directory) roles.
- Policy *actions* (trigger a compliance scan, create or extend exemptions, start
  remediation) come in M4 on top of the action framework.

## M4 — Operate (3 weeks)

- [ ] **Action framework.**
  - Confirmation dialog for every action. Typed-name confirmation for destructive
    ones.
  - `--readonly` flag.
  - A preview that shows the equivalent `az` command.
  - A local audit log of executed actions.
  - Status-bar tracking of long-running operations.
- [ ] **First actions:**
  - VM start, stop, deallocate and restart
  - App Service and Function App restart
  - Edit tags
  - Delete a resource or resource group
- [ ] **Policy actions:** trigger a compliance scan, create or extend an exemption,
      start remediation.
- [ ] **AKS basics:** clusters, node pools, `get-credentials`, and handing off to k9s.

## M5 — Rich views & polish (ongoing)

- Rich views for AKS, Key Vault (metadata only; secret values only on explicit
  request), Storage, VMs, and App Service log stream.
- Activity log per resource or resource group.
- Custom keybindings and themes.
- Packaging for Homebrew, winget, Scoop and AUR.

## M6 — Multi-cloud: c9s (estimate: ~13–19 working days, 7–10 PRs)

Started. az9s becomes **c9s**, *canines for clouds* (after k9s, canines for
Kubernetes), and browses GCP and AWS resources next to Azure. Azure keeps its depth (Policy, RBAC,
Entra ID); the other clouds get browsing.

Today about half of `c9s-core` (~15k lines) speaks Azure directly: the
`Subscription` / `ResourceGroup` / `Resource` models, the management group
tree, 21 Azure-shaped load requests and a cache keyed by subscription. The
`Fetcher` trait is the one clean seam.

How the clouds line up:

| Level | Azure | GCP | AWS |
|---|---|---|---|
| Context (`:ctx`) | tenant | organization / gcloud configuration | profile / organization |
| Tree | management groups | folders | organizational units |
| Account | subscription | project | account |
| Grouping | resource group | (location) | region |
| Inventory | Resource Graph | Cloud Asset Inventory (`searchAllResources`) | Resource Explorer; Tagging API fallback |
| Login | `az` CLI | `gcloud auth print-access-token` | `aws configure export-credentials` / SSO |

- [x] **Rename to c9s** (~0.5 day; v1.0.0). Crates, binary, config and cache
      directories (existing `~/.config/az9s` moved over), `AZ9S_*` variables
      (old names still read), release artifacts, docs, repository (GitHub
      redirects). A renamed binary is breaking: **c9s v1.0.0**, Azure only.
- [ ] **Provider abstraction in core** (~4–6 days; the main risk). *Step 1
      done (v1.1.0): `Provider` with each cloud's terms and capabilities;
      the status bar shows the context's cloud; titles use its words;
      Azure-only commands say so elsewhere. Found: one context at a time
      means rows need no provider field, so this is smaller than planned.
      Next: `:ctx` listing contexts per cloud, and the fetcher seam for a
      second cloud.* A generic
      resource (provider, account, container, type, location, tags/labels)
      and hierarchy tree; generic load requests (hierarchy, containers,
      resources, detail) beside an Azure-only set; capability checks so
      `:policy`, `:ra`, `:group`, `:app` say "Azure only" elsewhere; `:ctx`
      lists every installed CLI's contexts with a cloud column, and the
      status bar and breadcrumb start with it (`azure · Contoso`,
      `gcp · acme.com`); a cache migration adding the provider. No visible
      change for Azure; the existing tests guard it.
- [x] **GCP browsing** (~3–4 days; v1.2.0, preview). `:ctx` lists gcloud
      configurations beside Azure tenants; projects → resources (Cloud Asset
      `searchAllResources`, location as the container) → document. v1.3.0:
      folders as the tree (`:mg`), console links (`o`).
      Originally: Cloud Asset Inventory is close to Resource
      Graph (one paged search per organization, folder or project, with
      labels and locations); Resource Manager for the hierarchy.
- [x] **AWS browsing, single profiles** (v1.4.0, preview). `:ctx` lists the
      AWS CLI's profiles; a profile's account → resources (Resource
      Explorer in the profile's region; the Tagging API there without it;
      region as the container) → document; `o` opens the console by ARN.
      v1.5.0: Organizations (every account, OUs as the tree; each account
      through its own CLI profile, else an assumed role, `aws_role`).
      Originally: No inventory that is always
      on: Resource Explorer needs an aggregator index enabled by an admin;
      the Tagging API is per region and misses some untagged resources.
      Several accounts need Organizations (from the management account) and
      a role per account. Credentials from the `aws` CLI, requests signed by
      c9s (no AWS SDK).
- [x] **Polish** (v1.5.1). Per-cloud setup docs (`docs/CLOUDS.md`), help,
      starting without the Azure CLI; mock-API tests run in CI. Still open:
      trying GCP and AWS against real accounts.

Order: rename first, then the abstraction, then GCP (proves the abstraction
on the easier cloud), then AWS.

Open questions before starting:

1. ~~Name~~: **c9s**, decided (free on crates.io; three keys, no Shift).
   No `az9s` binary is shipped; config, cache and `AZ9S_*` variables carry
   over, and `alias az9s=c9s` keeps old habits working.
2. ~~AWS scope~~: **single profiles first**, decided; Organizations with a
   role per account may follow.
3. Azure-only features stay Azure-only (proposed): IAM views for GCP and
   AWS would be a separate, larger project.

---

## Decisions

| Decision | Choice | Status |
|---|---|---|
| Name | **c9s**, "canines for clouds" (binary, crates, data dir); az9s before v1.0.0 | **Decided** (done) |
| Who it's for | Platform engineers; open source | **Decided** |
| Product shape | Operations tool (not read-only browser) | **Decided** |
| Generic vs rich views | Generic for all types (M2). Rich views for AKS, VM, App Service/Functions, Key Vault, plus Policy | **Decided** |
| Policy scope | Management groups first-class; Policy (M3) before Operate (M4) | **Decided** |
| Auth | Azure CLI credential (`az login`) | **Decided** |
| Actions default | Enabled with confirmation; `--readonly` available | **Decided** |
| License | Apache-2.0 | **Decided** |
| Versioning | Plain SemVer per change: patch = fix, minor = feature, major = breaking; no pre-release tags; tags are never moved | **Decided** (done) |
| Command naming | Always singular (`:sub`, `:rg`, `:res`, `:mg`, …); no plural aliases | **Decided** (done) |
| Default scope | Tenant root management group | **Decided** |
| Read backend | Azure Resource Graph for lists, ARM for single-resource reads and writes | Proposed (validate in M1) |
| Multi-tenant | Switch tenants with `:ctx`; no parallel multi-tenant view | Done (v0.10.0) |
| Multi-cloud | Rename to c9s; browse GCP and AWS beside Azure (M6) | **Decided** (AWS scope still open, see M6) |
