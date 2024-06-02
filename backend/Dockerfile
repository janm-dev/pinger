# Builder image
FROM rust:latest AS builder

# Install musl libc for static linking
RUN rustup target add x86_64-unknown-linux-musl
RUN apt update && apt upgrade -y && apt install -y musl-tools musl-dev

# Create user
ENV USER=pinger
ENV UID=1001

RUN adduser \
	--disabled-password \
	--gecos "" \
	--home "/nonexistent" \
	--shell "/sbin/nologin" \
	--no-create-home \
	--uid "${UID}" \
	"${USER}"

WORKDIR /pinger

COPY ./ .

# Build with statically-linked musl libc
RUN cargo build --target x86_64-unknown-linux-musl --release

# Final image
FROM scratch

# Import from builder
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /pinger

# Copy the build
COPY --from=builder /pinger/target/x86_64-unknown-linux-musl/release/pinger-backend ./

# Use an unprivileged user
USER pinger:pinger

# Expose HTTP port
EXPOSE 80

ENTRYPOINT [ "/pinger/pinger-backend" ]
