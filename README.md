# СBLT

![cblt](https://github.com/evgenyigumnov/cblt/raw/HEAD/logo.png)

Safe and fast minimalistic web server, written in Rust, that serves files from a directory and proxies requests to another server.

## Project Name Explanation 

The name **Cblt** appears to be a good shortened version of **Cobalt**. It retains the association with the element and strength, while also looking modern and minimalist. 

## Features
- **10 times faster than Nginx for small content under 100KB**
- KDL Document Language configuration (**Cbltfile**)
- Proxy requests to another server
- Serve files from a directory
- TLS support
- Gzip compression
- Redirects

## Quick Start
You can run Cblt with Cargo or Docker.
### Cargo
Install:
```bash
cargo install cblt
```
Run:
```bash
cblt
```

### Docker
```bash
docker run -d -p 80:80 -p 443:443 --restart unless-stopped --name cblt ievkz/cblt
```


### Test

```
curl -H "Host: example.com"  http://127.0.0.1/
curl --insecure https:/127.0.0.1/
curl -X POST http://127.0.0.1/api/entry \
-H "User-Agent: curl/7.68.0" \
-H "Accept: */*" \
-H "Content-Type: application/json" \
-d '{"key":"value"}'
```

## "Cbltfile" configuration examples
### File server
```kdl
"*:80" {
    root "*" "/path/to/folder"
    file_server
}
```
### File server & Proxy
```kdl
"127.0.0.1:8080" {
    reverse_proxy "/test-api/*" "http://10.8.0.3:80"
    root "*" "/path/to/folder"
    file_server
}
```
### TLS support ([docs](https://github.com/evgenyigumnov/cblt/blob/main/tls.md))
```kdl
"example.com" {
    root "*" "/path/to/folder"
    file_server
    tls "/path/to/your/domain.crt" "/path/to/your/domain.key"
}
```
### Redirect
```kdl
"*:80" {
    redir "https://127.0.0.1{uri}"
}
```

## Benchmark
Do test with Apache Benchmark (ab) for 3000 requests with 1000 concurrent connections. Download 23kb image from 127.0.0.1/logo.png

```bash
 ab -c 1000 -n 3000 http://127.0.0.1/logo.png
``` 

| Percent | Cblt | Nginx |
|---------|------|-------|
| 50%     | 179 | 1209  |
| 75%     | 194 | 1655  |
| 100%    | 200 | 2146  |

## Contributing
I would love to see contributions from the community. If you experience bugs, feel free to open an issue. If you would like to implement a new feature or bug fix, please follow the steps:

1. Do fork
2. Do some changes
3. Create pull request

