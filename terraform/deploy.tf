###
#
# Deploy plumbing. The bootstrap + update workflows in `.github/workflows/`
# need:
#
#   - An SSH keypair whose public half is on the droplet (DO key) and whose
#     private half is exposed to GitHub Actions as a repo secret.
#   - The droplet's hostname.
#   - The proxy's Let's Encrypt contact email (for the templated proxy.toml).
#
# This file generates all three, so a fresh `terraform apply` from a wiped
# state yields a droplet you can immediately deploy to.
#
###

resource "tls_private_key" "deploy_testbed" {
  algorithm = "ED25519"
}

resource "digitalocean_ssh_key" "deploy_testbed" {
  name       = "Deploy Testbed"
  public_key = tls_private_key.deploy_testbed.public_key_openssh
}

resource "github_actions_secret" "testbed_ssh_key" {
  repository      = var.github_repo
  secret_name     = "TESTBED_SSH_KEY"
  value = tls_private_key.deploy_testbed.private_key_openssh
}

resource "github_actions_secret" "testbed_host" {
  repository      = var.github_repo
  secret_name     = "TESTBED_HOST"
  value = "testbed.hubfederation.com"
}

# The proxy terminates TLS for `proxy.hubfederation.com`, distinct from
# the SSH target. Both DNS names resolve to the same droplet, but the
# Let's Encrypt cert must be issued for the proxy-facing one (which is
# what every install script and `~get-samizdat` URL hits).
resource "github_actions_secret" "proxy_domain" {
  repository  = var.github_repo
  secret_name = "PROXY_DOMAIN"
  value       = "proxy.hubfederation.com"
}

resource "github_actions_secret" "proxy_owner_email" {
  repository      = var.github_repo
  secret_name     = "PROXY_OWNER_EMAIL"
  value = var.proxy_owner_email
}

###
#
# Deploy key for the `get-samizdat` release-collection repo. The
# `build-artifacts.yaml` workflow recursively clones the
# `install/get-samizdat` submodule and `postbuild.sh` pushes new
# release commits into it. Both need credentials.
#
# Using a deploy key (repo-scoped) rather than a user PAT (account-
# scoped) keeps the blast radius small: this key can read/write
# get-samizdat and nothing else.
#
###

resource "tls_private_key" "get_samizdat_deploy" {
  algorithm = "ED25519"
}

resource "github_repository_deploy_key" "get_samizdat" {
  repository = var.get_samizdat_repo
  title      = "samizdat-builder"
  key        = trimspace(tls_private_key.get_samizdat_deploy.public_key_openssh)
  read_only  = false
}

resource "github_actions_secret" "get_samizdat_deploy_key" {
  repository  = var.github_repo
  secret_name = "GET_SAMIZDAT_DEPLOY_KEY"
  value       = tls_private_key.get_samizdat_deploy.private_key_openssh
}
