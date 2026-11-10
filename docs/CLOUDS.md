# Clouds: setup and permissions

c9s shows one context at a time: an Azure tenant, a gcloud configuration
or an AWS CLI profile. `:ctx` lists them all; `Enter` (or `:ctx <name>`)
switches, and the status bar names the context (`azure · Contoso`,
`gcp · work`, `aws · prod-admin`). Each cloud signs in through its own CLI;
c9s stores no credentials.

c9s only reads. Every permission below is a read permission.

| | Azure | Google Cloud | AWS |
|---|---|---|---|
| Context | tenant | gcloud configuration | CLI profile |
| Sign in | `az login` | `gcloud auth login` | `aws sso login` or `aws configure` |
| Tree (`:mg`) | management groups | organization and folders | organization and units |
| Accounts | subscriptions | projects | accounts |
| Group in an account | resource group | location | region |
| Inventory | Resource Graph | Cloud Asset Inventory | Resource Explorer (Tagging API without it) |
| `o` opens | Azure Portal | Google Cloud console | AWS console |
| Policy, RBAC, Entra ID, cost | yes | Azure only | Azure only |

Without the Azure CLI, c9s starts on `:ctx`, so you can use it for Google
Cloud or AWS alone.

## Azure

1. Install the [Azure CLI](https://learn.microsoft.com/cli/azure/install-azure-cli)
   and run `az login`.
2. **Reader** on the subscriptions (or on a management group above them)
   lists them and their resources.
3. The management group tree needs **Management Group Reader** at the
   tenant root; without it c9s shows the subscriptions.
4. PIM eligible role assignments come from ARM
   (`roleEligibilityScheduleInstances`): Reader at the tenant root shows
   them all, else c9s reads each subscription it can.
5. Policy views read compliance through Resource Graph (Reader is enough).
   Group and application names come from Microsoft Graph and need the
   directory read access members have by default.
6. `:cost` reads Cost Management (`Microsoft.CostManagement/query`) and
   needs **Cost Management Reader** (or Reader) on the scope. Not every
   agreement type supports every scope (management groups in particular);
   when one doesn't, c9s shows Azure's message. Cost Management throttles queries: c9s
   retries when asked to wait and keeps each report for the session, so
   switching back (or switching currency) asks nothing.

## Google Cloud

1. Install the [Google Cloud CLI](https://cloud.google.com/sdk/docs/install)
   and run `gcloud auth login`. Each `gcloud config configurations` entry
   is a context.
2. Enable the Cloud Asset API once per project:
   `gcloud services enable cloudasset.googleapis.com --project <project>`.
3. Roles:
   - **Browser** (`roles/browser`) lists projects and folders;
   - **Cloud Asset Viewer** (`roles/cloudasset.viewer`) on the projects (or
     a folder or the organization above them) lists their resources;
   - **Folder Viewer** (`roles/resourcemanager.folderViewer`) on the
     organization shows the folders in `:mg`. Without it, `:mg` lists
     projects under one root.

## AWS

1. Install [AWS CLI v2](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html)
   and sign in (`aws sso login --profile <profile>`, or `aws configure`).
   c9s reads the profiles from `~/.aws/config` and `~/.aws/credentials`
   (or `AWS_CONFIG_FILE` and `AWS_SHARED_CREDENTIALS_FILE`) and asks the
   CLI for their credentials.
2. Resources come from
   [Resource Explorer](https://docs.aws.amazon.com/resource-explorer/latest/userguide/getting-started-setting-up.html),
   searched in the profile's region (`us-east-1` if it names none). Turn it
   on with an aggregator index in that region to see every region. Without
   it, c9s uses the Tagging API in that region and says so; untagged
   resources may be missing. A search returns 1,000 resources at most.
3. Permissions for each account:
   - `resource-explorer-2:Search` (or `tag:GetResources` for the fallback);
   - `sts:GetCallerIdentity`, which every principal has.
4. **Organizations.** With a profile in the management account (or a
   delegated administrator), c9s lists every account and shows the units
   in `:mg`. That needs `organizations:ListRoots`,
   `organizations:ListOrganizationalUnitsForParent` and
   `organizations:ListAccountsForParent`. c9s reaches each other account:
   - through a CLI profile that signs in to it (an SSO profile with that
     `sso_account_id`, or a `role_arn` in it), else
   - by assuming `OrganizationAccountAccessRole` in it (needs
     `sts:AssumeRole` on that role). Name another role with `aws_role` in
     the config file.

   Accounts it can't reach are reported; the rest are still listed.

## Not yet tried against real accounts

The Google Cloud and AWS support is a preview: it is tested against mock
APIs, not yet against real projects and accounts. Reports of what works
and what doesn't are welcome in the
[issues](https://github.com/mrWiecek/c9s/issues).
