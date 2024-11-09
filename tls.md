# For windows users

```bash
scoop install cmake
```
# Self-signed certificate
```bash
rm -f domain.key domain.crt
openssl req -x509 -nodes -newkey rsa:4096 \
-keyout domain.key \
-out domain.crt \
-days 365 \
-config self_signed.conf
```



**Cdlfile** example
```kdl
"127.0.0.1" {
    root "*" "/path/to/folder"
    file_server
    tls "/path/to/your/certificate.crt" "/path/to/your/private.key"
}
```

# Domain specific certificate
// TODO