resource "digitalocean_droplet" "samizdat_testbed" {
  name          = "samizdat-testbed"
  image         = "ubuntu-22-04-x64"
  size          = "s-1vcpu-1gb"
  region        = "nyc3"
  droplet_agent = true
  ipv6          = true
  monitoring    = true
  user_data     = file("resources/droplet-config.yaml")
  ssh_keys = [
    "31459399", # "Deploy Testbed" 
    "31256334", # "acer-nitro5-garuda-linux"
  ]
}
