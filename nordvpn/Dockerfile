FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y \
    wget \
    iproute2 iptables libatm1 libbpf0 libbsd0 libcap2 libcap2-bin libedit2 \
    libelf1 libgpm2 libicu67 libip4tc2 libip6tc2 libjansson4 libmd0 libmnl0 \
    libncurses6 libncursesw6 libnetfilter-conntrack3 libnfnetlink0 libnftables1 \
    libnftnl11 libpam-cap libprocps8 libxml2 libxtables12 netbase nftables \
    procps psmisc \
    && rm -rf /var/lib/apt/lists/*
RUN bash -c "export DEBIAN_FRONTEND=noninteractive; alias apt-get='apt-get --assume-yes'; bash <(wget -qO - https://downloads.nordcdn.com/apps/linux/install.sh)"
CMD ["nordvpn"]
