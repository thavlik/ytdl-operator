ARG BASE_IMAGE=python:3.8-slim-bullseye

FROM ${BASE_IMAGE} AS builder

# Install some basic dependencies. Since we're using a slim
# image, we can't GET download random URLs out of the box.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        curl \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Download the latest version of yt-dlp.
# It's not worth versioning this because youtube updates their
# internal API frequently enough that it's guaranteed to break.
RUN curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp \
    && chmod a+rx /usr/local/bin/yt-dlp

FROM ${BASE_IMAGE}
COPY --from=builder /usr/local/bin/yt-dlp /usr/local/bin/yt-dlp
CMD ["yt-dlp"]
