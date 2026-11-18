# A typical Terraform module, used by `bailiwick import terraform` and the
# import tests: an AWS API instance and worker, an RDS database, an S3
# bucket, a GCP instance, and resources BWK has no use for.

terraform {
  required_providers {
    aws = { source = "hashicorp/aws", version = "~> 5.0" }
  }
}

provider "aws" {
  region = "eu-central-1"
}

variable "instance_type" {
  type    = string
  default = "t3.medium"
}

resource "aws_instance" "api" {
  ami           = data.aws_ami.ubuntu.id
  instance_type = var.instance_type
  subnet_id     = aws_subnet.private.id

  user_data = <<-EOF
    #!/bin/bash
    echo "DATABASE_URL=${aws_db_instance.main.address}" >> /etc/environment
  EOF

  tags = {
    Name = "api"
    Team = "shop"
  }
}

resource "aws_db_instance" "main" {
  identifier          = "shop-main"
  engine              = "postgres"
  engine_version      = "16"
  instance_class      = "db.t3.medium"
  allocated_storage   = 50
  multi_az            = true
  skip_final_snapshot = false
}

resource "aws_s3_bucket" "media" {
  bucket = "acme-shop-media"
}

resource "aws_instance" "worker" {
  ami           = data.aws_ami.ubuntu.id
  instance_type = "t3.small"
  depends_on    = [aws_db_instance.main, aws_s3_bucket.media]
  tags          = { Name = "worker" }
}

resource "google_compute_instance" "reports" {
  name         = "reports"
  machine_type = "e2-standard-2"
  zone         = "europe-west3-a"
  boot_disk {
    initialize_params {
      image = "debian-cloud/debian-12"
    }
  }
  network_interface {
    network = "default"
  }
}

resource "aws_security_group" "api" {
  name = "api"
  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }
}
