FROM scratch
#当使用systemd来验证时，需要删除注释
#RUN rm -f /sbin/init
COPY target/x86_64-unknown-linux-musl/debug/init /sbin/init
COPY target/x86_64-unknown-linux-musl/debug/process1 /usr/lib/process1/process1
CMD ["/sbin/init"]
