# Overview

This repository provides a minimal reproducer for an issue encountered when
connecting to Cloudflare's 1.1.1.1 DNS resolver via DNS over HTTP/3 with the
`quinn` Rust crate.

Cloudflare's [documentation][cf-docs] demonstrates how to send queries to
1.1.1.1 using DoH, including examples using curl. There are multiple ways to
send a query, including sending a DNS message as a GET query parameter, sending
a DNS message as a POST request body, or sending a JSON object as a GET query
parameter. Aside from DoH, DoT is also supported.

[cf-docs]: https://developers.cloudflare.com/1.1.1.1/encryption/dns-over-https/make-api-requests/dns-wireformat/#using-post

Hickory DNS developers have noticed that 1.1.1.1 returns 400 Bad Request in
response to queries sent as a POST request body over HTTP/3, using the `quinn`
Rust crate. The same problem does not happen when using HTTP/2, or when passing
the query as a GET query parameter.

Disabling HTTP/3 GREASE and adding a Content-Length header results in a
successful request. Different error messages are returned when the
Content-Length header is omitted or GREASE is included. It appears that
including GREASE results in the HEADERS and DATA HTTP/3 frames being placed in
separate packets, which may impact how the 1.1.1.1 server handles the request.
[Other testing][steffengy-comment] has found that Cloudflare servers can handle
GREASE frames before DATA frames, but cannot handle GREASE frames after DATA
frames.

[steffengy-comment]: https://github.com/hyperium/h3/issues/206#issuecomment-2617014977

# Rust reproducer

The Rust package in this repository builds a single executable that makes DoH
requests that demonstrate this issue. Support for SSLKEYLOGFILE is included, in
order to inspect packet captures.

```sh
RUST_LOG=debug SSLKEYLOGFILE=keylogs/quinn.txt cargo run
```

# Curl comparison

Similar requests made with curl are successful, using either quiche or nghttp3.

```sh
docker build -f Dockerfile.debian-package -t curl-ngtcp2-nghttp3 .
docker run --rm -i curl-ngtcp2-nghttp3 curl --version
docker run --rm -v $PWD/keylogs:/keylogs -e SSLKEYLOGFILE=/keylogs/curl-ngtcp2-nghttp3.txt curl-ngtcp2-nghttp3 curl --http3-only --header 'content-type: application/dns-message' --data-binary @request.bin https://cloudflare-dns.com/dns-query --output - | xxd

docker build -f Dockerfile.boringssl-quiche -t curl-quiche .
docker run --rm -i curl-quiche curl --version
docker run --rm -v $PWD/keylogs:/keylogs -e SSLKEYLOGFILE=/keylogs/curl-quiche.txt curl-quiche curl --http3-only --header 'content-type: application/dns-message' --data-binary @request.bin https://cloudflare-dns.com/dns-query --output - | xxd
```
