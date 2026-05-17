resource "digitalocean_droplet" "samizdat_testbed" {
  name          = "samizdat-testbed"
  image         = "ubuntu-22-04-x64"
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
}
