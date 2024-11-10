```bash
docker build -t cblt:experemental .
docker run -d -p 80:80 --restart unless-stopped --name cblt cblt:experemental
```
