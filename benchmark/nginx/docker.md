```bash
docker build -t nginx_srv .
docker run -d -p 80:80 --restart unless-stopped --name nginx_srv nginx_srv
```
