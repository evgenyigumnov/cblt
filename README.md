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
| 50%     | 438  | 1974  | 0     |
| 100%    | 442  | 2068  | 0     |

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
Time taken for tests:   6.169 seconds
Complete requests:      300
Failed requests:        0
Total transferred:      1536804300 bytes
HTML transferred:       1536732300 bytes
Requests per second:    48.63 [#/sec] (mean)
Time per request:       2056.234 [ms] (mean)
Time per request:       20.562 [ms] (mean, across all concurrent requests)
Transfer rate:          243290.27 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.3      0       2
Processing:  1330 1979 226.9   1973    2523
Waiting:        2  140 129.2     80     576
Total:       1331 1979 226.8   1974    2523

Percentage of the requests served within a certain time (ms)
  50%   1974
  66%   2068
  75%   2150
  80%   2207
  90%   2277
  95%   2347
  98%   2412
  99%   2466
 100%   2523 (longest request)
```