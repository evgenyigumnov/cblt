# Static tests
```bash
ab -c 100 -n 300  http://127.0.0.1/logo_huge.png
```

# Reverse proxy tests
```bash
ab -c 100 -n 300 -p post_data.json -T "application/json" http://127.0.0.1/api/entry
```