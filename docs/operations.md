# Operating the testbed

A runbook for the things you actually have to do to keep the public
testbed (`testbed.hubfederation.com` / `proxy.hubfederation.com`)
alive, not a description of what the code is. For the latter, see
[`extras.md`](extras.md).

## Mental model

The testbed is a single DigitalOcean droplet running three Samizdat
daemons (hub, node, proxy) plus an opinionated cloud-init. It serves
two roles at once:

1. **The federation seed.** Every Linux `install.sh` does
   `samizdat hub new testbed.hubfederation.com UseBoth`, so every new
   node in the wild peers with this box.
2. **The release-distribution origin.** The testbed's node holds the
   `get-samizdat` collection (series key
   `r0Km0HptEt6Fhosmy7qxaKxyDtwHkzi0-eYbt1WatdM`). The proxy at
   `proxy.hubfederation.com` exposes that collection over HTTPS, which
   is where the install scripts curl their binaries from.

Releases and updates are dogfooded: after first bootstrap, the testbed
itself updates by pulling new binaries from the very `get-samizdat`
collection it serves. The only out-of-band path is the **first** seed.

## Bootstrap vs update: why two workflows

A fresh droplet has nothing samizdat-shaped on it, so the first
install must come from somewhere outside the samizdat network --
there is no samizdat network yet that contains a binary the droplet
can run. After that single seed, the testbed participates in its own
distribution and can self-update.

- **`bootstrap-testbed.yaml`** is the one-shot out-of-band seed. It
  builds Linux x86_64 binaries on a GitHub runner, scp's them onto
  the droplet with configs and systemd units, starts the services,
  and subscribes the testbed's node to both `get-samizdat` and the
  `samizdat-blog` collections. Run after `terraform apply`, once per
  droplet recreation.
- **`update-testbed.yaml`** is the recurring update. It ssh's into
  the droplet and runs `samizdat-self-update`, which pulls binaries
  from `localhost:4510/~get-samizdat/latest/...` (i.e. from the
  testbed's own copy of the collection) and atomically restarts
  the services. Run after publishing a new edition of `get-samizdat`
  from your laptop.

That asymmetry is on purpose. Resist the urge to make bootstrap also
"pull from the net" -- you'd just be moving the chicken-and-egg
somewhere else.

## What is and isn't managed by Terraform

### Managed

- The droplet (size, image, region, ipv6, monitoring).
- The DigitalOcean SSH key used by the deploy workflows (generated
  fresh by `tls_private_key` on every state reset).
- The `digitalocean_firewall` is **not** managed; UFW on the droplet
  is. See "Firewall ports" below.
- The `hubfederation.com` domain and the four DNS records
  (`testbed` + `proxy`, A + AAAA).
- The `samizdat` DigitalOcean project, grouping the droplet + domain.
- GitHub Actions secrets: `TESTBED_SSH_KEY`, `TESTBED_HOST`,
  `PROXY_DOMAIN`, `PROXY_OWNER_EMAIL`, `GET_SAMIZDAT_DEPLOY_KEY`.
- The `samizdat-builder` deploy key on the `get-samizdat` repo (read +
  write, scoped to that one repo).

### Not managed

- **Your personal SSH key on the droplet.** Add it through the DO
  console after `terraform apply`. Keeping it out of TF means
  `terraform destroy && apply` from a different developer's machine
  still works.
- **The `.Samizdat.priv` for the `get-samizdat` collection.** Lives
  only at `install/get-samizdat/.Samizdat.priv` on your workstation,
  gitignored, never in TF, never on the droplet, never in CI.
  Backing it up is your responsibility: if it's gone, every existing
  user's `subscription new r0Km0Hpt...` is orphaned and the
  collection identity has to be reissued.
- **The DO API token, GitHub PAT, and proxy owner email.** Set in
  `terraform/terraform.auto.tfvars` (gitignored). When the DO token
  expires, drop a fresh one there and re-apply.

## Recreate from scratch

When the droplet is gone (destroyed, lost, you tainted it on
purpose), here is the full ceremony:

1. Confirm `.Samizdat.priv` is on your workstation at
   `install/get-samizdat/.Samizdat.priv`. If not, your update path is
   dead until you mint a new collection and re-publish.
2. Make sure `terraform/terraform.auto.tfvars` has working values
   for `do_token`, `github_token`, `github_owner`,
   `proxy_owner_email`. The DO token tends to be the one that
   expires; rotate via DigitalOcean console, paste in, done.
3. `cd terraform && terraform apply -auto-approve`. Resources
   created: droplet + 4 DNS records + DO project + DO SSH key + the
   five GH action secrets + the get-samizdat deploy key.
4. Wait for cloud-init to finish on the new droplet. Pretty quick
   (apt update + apt upgrade + a few `mkdir -p`s); 2-5 minutes from
   `apply` completion.
5. Dispatch bootstrap:
   `gh workflow run bootstrap-testbed.yaml --ref <branch> -f ref=<branch>`.
   First run takes 8-15 minutes (full Rust compile from scratch on
   the runner). Subsequent runs from the same lockfile reuse the
   shared cargo cache.
6. Probe:
   - `curl -fsSI https://proxy.hubfederation.com/` should return a
     valid cert (issued for `proxy.hubfederation.com`) and a 4xx/5xx
     status until you publish content.
   - `nc -uz testbed.hubfederation.com 4511` (UDP) should succeed.
   - `ssh root@testbed.hubfederation.com 'systemctl is-active samizdat-hub samizdat-node samizdat-proxy'`
     should print `active` three times.
7. From your laptop, publish a fresh edition of `get-samizdat`:
   `cd install/get-samizdat && samizdat collection update`. The
   testbed's subscription picks it up on the next refresh tick.
8. Once the edition is in, run `gh workflow run update-testbed.yaml`
   to flip the testbed onto its own published binaries.

If anything in step 5 fails, the failure mode is usually one of the
gotchas in the next section.

## Gotchas (the things that bit me at least once)

- **`user_data` is ForceNew on `digitalocean_droplet`.** Editing
  `terraform/resources/droplet-config.yaml` would silently destroy
  and recreate the droplet on the next `apply`, churning the IP and
  the Let's Encrypt account. The droplet resource pins this with
  `lifecycle { ignore_changes = [user_data] }`. To intentionally
  apply a new cloud-init, `terraform taint
  digitalocean_droplet.samizdat_testbed` first. Cloud-init only runs
  on the **first** boot anyway, so editing it without recreating the
  droplet does nothing to the running box; treat the file as
  "instructions for the next droplet."
- **TF Cloud auto-loads `terraform.tfvars` and `*.auto.tfvars`, NOT
  plain `.tfvars`.** Earlier this repo had `terraform/.tfvars` which
  was effectively dead weight (the file was uploaded but not auto-
  loaded). The current name `terraform/terraform.auto.tfvars` is the
  one TF actually reads.
- **TF Cloud workspace version pin must match your local.** If
  workspace says `~> 1.5.0` and you run `terraform 1.13`, remote
  plans and applies work but local commands like `terraform import`
  fail. Bump the workspace version in TF Cloud settings.
- **`github_actions_secret` uses `value` in provider v6+** (was
  `plaintext_value` in earlier versions). Easy to miss across a
  major-version provider bump.
- **`gh workflow run` only finds workflows on the default branch.**
  A workflow file that exists only on a feature branch is invisible
  to `gh workflow run` until you also land it on `main`. The
  workflow execution itself uses the file from `--ref`, not from
  `main`, so the canonical version can live on the feature branch;
  you just need a stub on main for discovery.
- **Login shell on the droplet is fish.** Non-interactive `ssh
  host '...'` runs the command through fish, which won't parse bash
  syntax (`for/do/done`, `set -e`, etc.). The workflows pipe
  multi-line scripts to `bash -s` on the remote
  (`ssh host bash -s <<'EOF' ... EOF`), and single-command
  invocations work fine because fish just exec's the binary.
- **`PROXY_DOMAIN` is not the same as `TESTBED_HOST`.** Both DNS
  labels (`testbed.hubfederation.com`, `proxy.hubfederation.com`)
  resolve to the same droplet. The proxy's Let's Encrypt cert is
  issued for `PROXY_DOMAIN` because that's where install scripts
  point; SSH targets `TESTBED_HOST`. Easy to conflate; the workflow
  uses both deliberately.
- **The `get-samizdat` submodule clone needs the deploy key.**
  Workflows that recursively check out submodules will 404 unless
  the `GET_SAMIZDAT_DEPLOY_KEY` secret is installed into the SSH
  agent before checkout. `bootstrap-testbed.yaml` doesn't need it
  (it builds from the workspace and skips submodules);
  `build-artifacts.yaml` does need it (postbuild.sh pushes new
  releases into get-samizdat).

## Firewall ports

UFW on the droplet, configured by cloud-init:

| Port      | Protocol | Why                                    |
|-----------|----------|----------------------------------------|
| 22        | tcp      | SSH for management                     |
| 80        | tcp      | HTTP -> HTTPS redirect from the proxy  |
| 443       | tcp      | HTTPS, served by the proxy             |
| 4511      | udp      | The hub's QUIC accept loop             |

The hub's HTTP admin port (`45180/tcp`) is intentionally NOT exposed.
If you need it, tunnel via SSH:
`ssh -L 45180:localhost:45180 root@testbed.hubfederation.com`.

## Where to look when things break

- **Service not running**: `ssh root@testbed.hubfederation.com
  'systemctl status samizdat-{hub,node,proxy}'` and `journalctl -u
  samizdat-<x> -n 200`.
- **TLS cert wrong**: check `PROXY_DOMAIN` matches the URL you're
  testing. `journalctl -u samizdat-proxy | grep -i acme` for Let's
  Encrypt errors. Rate limits and ACME state live under
  `/var/lib/samizdat/proxy/acme/`.
- **install.sh fails on a user's machine**: confirm
  `curl -fsSL https://proxy.hubfederation.com/~get-samizdat/latest/x86_64-unknown-linux-gnu/node/samizdat-node | wc -c`
  returns a binary-sized number. If 0/HTML/404, the testbed's node
  doesn't have the current edition -- republish from your laptop or
  refresh the subscription on the testbed.
- **Workflow can't dispatch**: it isn't on `main`. Cherry-pick the
  workflow file onto main.
- **terraform apply 401s on DO**: token expired. Rotate via DO
  console, update `terraform/terraform.auto.tfvars`, re-apply.
