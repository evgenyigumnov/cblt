```bash
docker build -t caddy_srv .
docker run -d -p 80:80 --restart unless-stopped --name caddy_srv caddy_srv
```
