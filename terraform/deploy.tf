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
  plaintext_value = tls_private_key.deploy_testbed.private_key_openssh
}

resource "github_actions_secret" "testbed_host" {
  repository      = var.github_repo
  secret_name     = "TESTBED_HOST"
  plaintext_value = "testbed.hubfederation.com"
}

resource "github_actions_secret" "proxy_owner_email" {
  repository      = var.github_repo
  secret_name     = "PROXY_OWNER_EMAIL"
  plaintext_value = var.proxy_owner_email
}
