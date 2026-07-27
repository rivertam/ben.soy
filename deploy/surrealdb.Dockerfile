FROM surrealdb/surrealdb:v3.2.3

# Attach the Railway volume at /home/nonroot. Railway supplies the initial
# root credentials through SURREAL_USER and SURREAL_PASS. Railway mounts
# volumes as root, so the database process must run as root to write RocksDB.
USER root

ENV SURREAL_BIND=0.0.0.0:8000 \
    SURREAL_PATH=rocksdb:///home/nonroot/data.db

EXPOSE 8000
HEALTHCHECK --interval=5s --timeout=3s --start-period=5s --retries=12 \
    CMD ["/surreal", "is-ready", "--endpoint", "http://127.0.0.1:8000"]
CMD ["start"]
