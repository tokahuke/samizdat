terraform {
  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "~> 2.0"
    }
  }

  cloud {
    organization = "samizdat"

    workspaces {
      name = "Samizdat"
    }
  }
}

variable "do_token" {
  type = string
}

# Configure the DigitalOcean Provider
provider "digitalocean" {
  token = var.do_token
}
