# СBLT

![cblt](https://github.com/evgenyigumnov/cblt/raw/HEAD/logo.png)

Safe and fast minimalistic web server, written in Rust, that serves files from a directory.

## Project Name Explanation 

The name **Cblt** appears to be a good shortened version of **Cobalt**. It retains the association with the element and strength, while also looking modern and minimalist. 


## Docker
```bash
docker build -t cblt:0.0.1 .
docker run -d -p 80:80 --restart unless-stopped --name cblt cblt:0.0.1
```
## Test

curl -H "Host: example.com"  http://127.0.0.1/Cargo.toml


## Benchmark
| Percent | Cblt | Nginx | Caddy |
|---------|------|-------|-------|


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
Time taken for tests:   1.328 seconds
Complete requests:      300
Failed requests:        0
Total transferred:      1536745500 bytes
HTML transferred:       1536732300 bytes
Requests per second:    225.83 [#/sec] (mean)
Time per request:       442.804 [ms] (mean)
Time per request:       4.428 [ms] (mean, across all concurrent requests)
Transfer rate:          1129716.65 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.4      0       2
Processing:   408  436  11.4    438     463
Waiting:        0    6   3.4      4      16
Total:        408  436  11.4    438     463

Percentage of the requests served within a certain time (ms)
  50%    438
  66%    442
  75%    445
  80%    446
  90%    450
  95%    455
  98%    458
  99%    461
 100%    463 (longest request)
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
igumn@lenovo MINGW64 ~/cblt/benchmark/nginx (main)
$ docker ps
CONTAINER ID   IMAGE                 COMMAND                  CREATED              STATUS                  PORTS                                                       NAMES
b569d15912db   caddy_srv             "caddy run --config …"   About a minute ago   Up About a minute       443/tcp, 0.0.0.0:80->80/tcp, 2019/tcp, 443/udp              caddy_srv
fe5e452458a1   syncthing/syncthing   "/bin/entrypoint.sh …"   2 days ago           Up 13 hours (healthy)   21027/udp, 127.0.0.1:8384->8384/tcp, 22000/udp, 22000/tcp   syncthing

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


Server Software:
Server Hostname:        example.com
Server Port:            80

Document Path:          /logo_huge.png
Document Length:        5122441 bytes

Concurrency Level:      100
Time taken for tests:   1.311 seconds
Complete requests:      300
Failed requests:        0
Total transferred:      1536745500 bytes
HTML transferred:       1536732300 bytes
Requests per second:    228.84 [#/sec] (mean)
Time per request:       436.994 [ms] (mean)
Time per request:       4.370 [ms] (mean, across all concurrent requests)
Transfer rate:          1144735.80 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.3      0       2
Processing:   374  432  14.9    429     466
Waiting:        1    7   3.4      6      20
Total:        374  432  14.9    429     466

Percentage of the requests served within a certain time (ms)
  50%    429
  66%    436
  75%    443
  80%    446
  90%    453
  95%    457
  98%    462
  99%    464
 100%    466 (longest request)
```