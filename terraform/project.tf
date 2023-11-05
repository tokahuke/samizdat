resource "digitalocean_project" "project" {
  name        = "samizdat"
  description = "Your content, available."
  purpose     = "Distributed system testbed"
  environment = "Production"
  resources = [
    resource.digitalocean_droplet.samizdat_testbed.urn,
    resource.digitalocean_domain.hubfederation.urn
  ]
}
