FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release

FROM alpine:3.20
COPY --from=builder /app/target/release/pensieve-container /pensieve-container
EXPOSE 8080
ENTRYPOINT ["/pensieve-container"]
