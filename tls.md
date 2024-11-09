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
