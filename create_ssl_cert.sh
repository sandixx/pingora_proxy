#!/bin/bash

# Create SSL directory if it doesn't exist
mkdir -p ssl

# Generate private key
openssl genrsa -out ssl/server.key 2048

# Generate self-signed certificate
openssl req -new -x509 -sha256 -key ssl/server.key -out ssl/server.pem -days 3650 -subj "/C=US/ST=State/L=City/O=Organization/CN=localhost"

echo "SSL certificate generated:"
echo "  Key: ssl/server.key"
echo "  Cert: ssl/server.pem"