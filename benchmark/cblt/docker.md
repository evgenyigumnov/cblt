```bash
docker build -t cblt:0.0.1 .
docker run -d -p 80:80 --restart unless-stopped --name cblt cblt:0.0.1
```
