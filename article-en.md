# CBLT — A Safe, Fast, and Minimalistic Web Server in Rust Programming Language

To learn a new programming language, I use the following approach. First, I read a tutorial for that programming language, which explains the syntax, idioms, philosophy, and principles of how the language works. After that, I write a small pet project in that programming language. In the pet project, I practice a bit with the new language, its standard libraries, and popular frameworks.

To immerse myself deeper into the language, instead of a pet project, I start writing my own libraries for working with databases (ORM), JSON, actors, MVC web frameworks, logging, etc. These libraries are unlikely to be needed by anyone, but they help me better understand the programming language. Surprisingly, with Rust, I managed to write my own web server. I hadn't done this before. I think it's because Rust is a systems programming language, and it's not a sin to try optimizing performance with it.

In the end, I encountered that Rust does not have equivalents to Nginx, Lighttpd, Caddy, HAProxy, Apache, Tomcat, Jetty, etc. All these web servers are written in C, Go, Java, etc. There are only web frameworks: Actix, Axum, Rocket, Hyper, etc.

Overall, I figured out that I usually use Nginx for the following purposes:

1. TLS for domains
2. Proxying requests to the backend
3. Serving static files

As a result, I decided to write my own implementation of a web server in Rust.

The server supports a configuration file in the KDL (KDL Document Language) format. Here are examples of the "Cbltfile" configuration file for the Cblt web server:

**Server File**
```kdl
"*:80" {
    root "*" "/path/to/folder"
    file_server
}
```

**File Server & Proxying**
```kdl
"127.0.0.1:8080" {
    reverse_proxy "/test-api/*" "http://10.8.0.3:80"
    root "*" "/path/to/folder"
    file_server
}
```

**TLS Support**
```kdl
"example.com" {
    root "*" "/path/to/folder"
    file_server
    tls "/path/to/your/domain.crt" "/path/to/your/domain.key"
}
```

Currently, the Cblt web server can be launched in two ways: via Cargo or Docker.

***Cargo***
```bash
cargo run --release
```

***Docker***
```bash
docker build -t cblt:0.0.3 .
docker run -d -p 80:80 --restart unless-stopped --name cblt cblt:0.0.3
```

At the moment, I have achieved an acceptable speed for proxying static files.

I conducted a test with Apache Benchmark (ab) for 300 requests with 100 concurrent connections. Loading an image sized 5 MB from example.com/logo_huge.png.
```bash
ab -c 100 -n 300 http://example.com/logo_huge.png
```

| Percent | Cblt | Nginx | Caddy |
|---------|------|-------|-------|
| 50%     | 1956 | 1941  | 1768  |
| 75%     | 2101 | 2065  | 1849  |
| 100%    | 2711 | 2360  | 2270  |


### Cblt
```bash
igumn@lenovo MINGW64 ~/cblt (main)
$ docker ps
CONTAINER ID   IMAGE                 COMMAND                  CREATED         STATUS                 PORTS                                                       NAMES
0589d8f26d91   cblt:0.0.1            "./cblt"                 2 minutes ago   Up 2 minutes           0.0.0.0:80->80/tcp                                          cblt

igumn@lenovo MINGW64 ~/cblt (main)
$ ab -c 100 -n 300 http://example.com/logo_huge.png
This is ApacheBench, Version 2.3 <$Revision: 1913912 $>
Copyright 1996 Adam Twiss, Zeus Technology Ltd, http://www.zeustech.net/
Licensed to The Apache Software Foundation, http://www.apache.org/

Benchmarking example.com (be patient)
Completed 100 requests
Completed 200 requests
Completed 300 requests
Finished 300 requests


Server Software:
Server Hostname:        example.com
Server Port:            80

Document Path:          /logo_huge.png
Document Length:        5122441 bytes

Concurrency Level:      100
Time taken for tests:   6.020 seconds
Complete requests:      300
Failed requests:        0
Total transferred:      1536745500 bytes
HTML transferred:       1536732300 bytes
Requests per second:    49.83 [#/sec] (mean)
Time per request:       2006.721 [ms] (mean)
Time per request:       20.067 [ms] (mean, across all concurrent requests)
Transfer rate:          249283.62 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.3      0       2
Processing:  1293 1926 262.3   1956    2711
Waiting:        1  118 139.1     63     645
Total:       1293 1926 262.3   1956    2711

Percentage of the requests served within a certain time (ms)
  50%   1956
  66%   2027
  75%   2101
  80%   2127
  90%   2213
  95%   2394
  98%   2544
  99%   2597
 100%   2711 (longest request)
 ```

### Nginx

```bash
igumn@lenovo MINGW64 ~/cblt/benchmark/nginx (main)
$ docker ps
CONTAINER ID   IMAGE                 COMMAND                  CREATED         STATUS                  PORTS                                                       NAMES
37fbf1dac42b   nginx_srv             "/docker-entrypoint.…"   2 minutes ago   Up 2 minutes            0.0.0.0:80->80/tcp                                          nginx_srv

igumn@lenovo MINGW64 ~/cblt/benchmark/nginx (main)
$ ab -c 100 -n 300 http://example.com/logo_huge.png
This is ApacheBench, Version 2.3 <$Revision: 1913912 $>
Copyright 1996 Adam Twiss, Zeus Technology Ltd, http://www.zeustech.net/
Licensed to The Apache Software Foundation, http://www.apache.org/

Benchmarking example.com (be patient)
Completed 100 requests
Completed 200 requests
Completed 300 requests
Finished 300 requests


Server Software:        nginx/1.27.2
Server Hostname:        example.com
Server Port:            80

Document Path:          /logo_huge.png
Document Length:        5122441 bytes

Concurrency Level:      100
Time taken for tests:   6.043 seconds
Complete requests:      300
Failed requests:        0
Total transferred:      1536804300 bytes
HTML transferred:       1536732300 bytes
Requests per second:    49.65 [#/sec] (mean)
Time per request:       2014.267 [ms] (mean)
Time per request:       20.143 [ms] (mean, across all concurrent requests)
Transfer rate:          248359.28 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.3      0       2
Processing:  1387 1940 168.4   1941    2360
Waiting:        1  115  84.5     98     301
Total:       1387 1940 168.4   1941    2360

Percentage of the requests served within a certain time (ms)
  50%   1941
  66%   2024
  75%   2065
  80%   2088
  90%   2152
  95%   2201
  98%   2263
  99%   2317
 100%   2360 (longest request)
```

### Caddy

```bash
igumn@lenovo MINGW64 ~/cblt (main)
$ ab -c 100 -n 300 http://example.com/logo_huge.png
This is ApacheBench, Version 2.3 <$Revision: 1913912 $>
Copyright 1996 Adam Twiss, Zeus Technology Ltd, http://www.zeustech.net/
Licensed to The Apache Software Foundation, http://www.apache.org/

Benchmarking example.com (be patient)
Completed 100 requests
Completed 200 requests
Completed 300 requests
Finished 300 requests


Server Software:        Caddy
Server Hostname:        example.com
Server Port:            80

Document Path:          /logo_huge.png
Document Length:        5122441 bytes

Concurrency Level:      100
Time taken for tests:   5.440 seconds
Complete requests:      300
Failed requests:        0
Total transferred:      1536804000 bytes
HTML transferred:       1536732300 bytes
Requests per second:    55.14 [#/sec] (mean)
Time per request:       1813.469 [ms] (mean)
Time per request:       18.135 [ms] (mean, across all concurrent requests)
Transfer rate:          275858.99 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.3      0       2
Processing:  1264 1749 191.1   1767    2270
Waiting:        1   96 104.7     67     467
Total:       1265 1749 191.1   1768    2270

Percentage of the requests served within a certain time (ms)
  50%   1768
  66%   1821
  75%   1849
  80%   1877
  90%   1955
  95%   2152
  98%   2226
  99%   2241
 100%   2270 (longest request)
```



I also plan to conduct tests for backend proxying; in general, I haven't yet tested the reverse proxy for performance.

Maybe this time my mini-project will interest someone? And this hobby will grow into something bigger?

If you're interested in looking at the code or contributing, here's the link to the repository: [https://github.com/evgenyigumnov/cblt](https://github.com/evgenyigumnov/cblt)