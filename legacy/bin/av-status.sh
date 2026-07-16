#!/usr/bin/env bash
# waybar status for clamav antivirus (JSON)
if systemctl -q is-active clamav-daemon; then
    echo '{"text":"󰃤","class":"on","tooltip":"Antivirus (ClamAV): daemon running — click to scan"}'
else
    echo '{"text":"󰃤","class":"off","tooltip":"Antivirus (ClamAV): daemon NOT running — click for options"}'
fi
