FROM alpine:latest AS builder
COPY --chmod=0755 . /minecraft-pdb-mgr
RUN apk upgrade && \
    apk add curl clang lld && \
    ( curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y ) && \
    source ~/.cargo/env && \
    cd /minecraft-pdb-mgr && \
    RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld" cargo build --release
RUN <<_EOF_
    mkdir -p /out/libs
    mkdir -p /out/libs-root
    ldd /minecraft-pdb-mgr/target/release/minecraft-pdb-mgr
    ldd /minecraft-pdb-mgr/target/release/minecraft-pdb-mgr | grep -v 'linux-vdso.so' | awk '{print $(NF-1) " " $1}' | sort -u -k 1,1 | awk '{print "install", "-D", $1, (($2 ~ /^\//) ? "/out/libs-root" $2 : "/out/libs/" $2)}' | xargs -I {} sh -c {}
    ls -Rla /out/libs
    ls -Rla /out/libs-root
_EOF_

FROM scratch
COPY --chown=0:0 --chmod=0755 --from=builder /minecraft-pdb-mgr/target/release/minecraft-pdb-mgr /minecraft-pdb-mgr
COPY --from=builder /out/libs-root/ /
COPY --from=builder /out/libs/ /lib/
ENV LD_LIBRARY_PATH=/lib

ENV LC_ALL=C
LABEL org.opencontainers.image.authors=me@concord.sh

USER 1000:1000

ENTRYPOINT ["/minecraft-pdb-mgr"]
