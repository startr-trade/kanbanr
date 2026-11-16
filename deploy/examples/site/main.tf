# An example root for the kanbanr-site module (FTR-030): one site serving the
# boards listed in terraform.tfvars, with any credentials from the git-ignored
# terraform.sensitive.auto.tfvars. `tofu apply` (or `terraform apply`) loads
# both files by itself.
module "site" {
  source = "../../modules/kanbanr-site"

  name              = "site"
  namespace         = var.namespace
  image             = var.image
  ingress_host      = var.ingress_host
  boards            = var.boards
  board_credentials = var.board_credentials
}
