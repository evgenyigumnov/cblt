# СBLT

![cblt](https://github.com/evgenyigumnov/cblt/raw/HEAD/logo.png)

Safe and fast minimalistic web server, written in Rust, that serves files from a directory.

## Project Name Explanation 

The name **Cblt** appears to be a good shortened version of **Cobalt**. It retains the association with the element and strength, while also looking modern and minimalist. 


## Docker

docker build -t cblt:0.0.1 .
docker run -d -p 80:80 --restart unless-stopped --name cblt cblt:0.0.1

## Tests

curl -H "Host: example.com"  http://127.0.0.1/Cargo.toml


## Benchmark

```bash
igumn@lenovo MINGW64 ~/cblt (main)
$ docker ps
CONTAINER ID   IMAGE                 COMMAND                  CREATED         STATUS                 PORTS                                                       NAMES
0589d8f26d91   cblt:0.0.1            "./cblt"                 2 minutes ago   Up 2 minutes           0.0.0.0:80->80/tcp                                          cblt

igumn@lenovo MINGW64 ~/cblt (main)
$ ab -c 10 -n 200 http://example.com/logo_huge.png
This is ApacheBench, Version 2.3 <$Revision: 1913912 $>
Copyright 1996 Adam Twiss, Zeus Technology Ltd, http://www.zeustech.net/
Licensed to The Apache Software Foundation, http://www.apache.org/

Benchmarking example.com (be patient)
Completed 100 requests
Completed 200 requests
Finished 200 requests


Server Software:
Server Hostname:        example.com
Server Port:            80

Document Path:          /logo_huge.png
Document Length:        5122441 bytes

Concurrency Level:      10
Time taken for tests:   4.132 seconds
Complete requests:      200
Failed requests:        0
Total transferred:      1024497000 bytes
HTML transferred:       1024488200 bytes
Requests per second:    48.40 [#/sec] (mean)
Time per request:       206.609 [ms] (mean)
Time per request:       20.661 [ms] (mean, across all concurrent requests)
Transfer rate:          242119.88 [Kbytes/sec] received

Connection Times (ms)
              min  mean[+/-sd] median   max
Connect:        0    0   0.3      0       1
Processing:   145  205  16.8    207     243
Waiting:        3    9   2.6      9      22
Total:        145  205  16.8    207     243

Percentage of the requests served within a certain time (ms)
  50%    207
  66%    213
  75%    217
  80%    219
  90%    226
  95%    231
  98%    236
  99%    241
 100%    243 (longest request)
```