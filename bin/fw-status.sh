#!/usr/bin/env bash
# waybar status for ufw firewall (JSON)
if grep -q '^ENABLED=yes' /etc/ufw/ufw.conf 2>/dev/null && systemctl -q is-active ufw; then
    echo '{"text":"󰕥","class":"on","tooltip":"Firewall (ufw): ACTIVE — click for options"}'
else
    echo '{"text":"󰕥","class":"off","tooltip":"Firewall (ufw): OFF — click for options"}'
fi
