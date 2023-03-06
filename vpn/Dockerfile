FROM qmcgaw/gluetun:v3.32.0
RUN apk add --no-cache curl
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/bin/sh"]
CMD ["/entrypoint.sh"]
