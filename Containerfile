FROM docker.io/library/rust:1.88.0-alpine@sha256:63985230b69fbd90528857dabf261379eb47f285ccc69f577d17c3dfde721deb AS builder
COPY --chmod=0755 . /build
RUN apk update && \
    apk add clang lld && \
    export RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld" && \
    cd /build && \
    cargo build --release
RUN mkdir -p /out/libs && \
    mkdir -p /out/libs-root && \
    ldd /build/target/release/minecraft-pdb-mgr && \
    ldd /build/target/release/minecraft-pdb-mgr | grep -v 'linux-vdso.so' | awk '{print $(NF-1) " " $1}' | sort -u -k 1,1 | awk '{print "install", "-D", $1, (($2 ~ /^\//) ? "/out/libs-root" $2 : "/out/libs/" $2)}' | xargs -I {} sh -c {} && \
    ls -Rla /out/libs && \
    ls -Rla /out/libs-root

FROM scratch
COPY --chown=0:0 --chmod=0755 --from=builder /build/target/release/minecraft-pdb-mgr /minecraft-pdb-mgr
COPY --from=builder /out/libs-root/ /
COPY --from=builder /out/libs/ /lib/
ENV LD_LIBRARY_PATH=/lib

ENV LC_ALL=C
LABEL org.opencontainers.image.authors=me@concord.sh

USER 1000:1000

ENTRYPOINT ["/minecraft-pdb-mgr"]
