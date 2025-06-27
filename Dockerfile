FROM alpine:latest AS build
RUN apk add --no-cache rust cargo
WORKDIR /build
RUN mkdir src && echo 'fn main() {}' > src/main.rs
ADD Cargo.* ./
RUN cargo build -r
ADD src/* src/
RUN touch src/main.rs
RUN cargo build -r
FROM alpine:latest
RUN apk add --no-cache libgcc
COPY --from=build /build/target/release/quickshare /quickshare
USER 1000:1000
WORKDIR /data
EXPOSE 3000/tcp
CMD ["/quickshare"]
