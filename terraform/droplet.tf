resource "digitalocean_droplet" "samizdat_testbed" {
  name          = "samizdat-testbed"
  image         = "ubuntu-24-04-x64"
  size          = "s-1vcpu-1gb"
  region        = "nyc3"
  droplet_agent = true
  ipv6          = true
  monitoring    = true
  user_data     = file("resources/droplet-config.yaml")
  # Only the TF-managed Deploy Testbed key; the bootstrap workflow uses
  # it via the matching GH secret. Add personal keys out-of-band on the
  # DO console if you want a human login -- keeping them in TF would
  # tie destroy/apply to a specific developer machine.
  ssh_keys = [digitalocean_ssh_key.deploy_testbed.id]

  # `user_data` changes are ForceNew on the DO provider: editing
  # `resources/droplet-config.yaml` would destroy + recreate the
  # droplet, churning the IP and (if you don't notice) the bootstrap
  # workflow against a still-booting box. Cloud-init only runs at
  # first boot anyway, so ignoring this preserves the running droplet.
  # If you intend to apply a new cloud-init, `terraform taint` the
  # droplet explicitly.
  lifecycle {
    ignore_changes = [user_data]
  }
}
