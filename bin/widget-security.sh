#!/bin/sh
# Security posture card: firewall, antivirus, VPN, listening ports.
# --fields: key=value lines (+ _slot status inks) for hypr-cardhost.
if [ "$1" = "--fields" ]; then
    if grep -q "ENABLED=yes" /etc/ufw/ufw.conf 2>/dev/null; then
        echo "fw=● on";  echo "fw_slot=good"
    else
        echo "fw=● OFF"; echo "fw_slot=bad"
    fi
    if systemctl -q is-active clamav-daemon 2>/dev/null; then
        echo "av=● running"; echo "av_slot=good"
    else
        echo "av=● stopped"; echo "av_slot=sub"
    fi
    if ip -br link show up 2>/dev/null | grep -qE "^(tun|wg|proton|nordlynx)"; then
        echo "vpn=● connected"; echo "vpn_slot=good"
    else
        echo "vpn=● off"; echo "vpn_slot=sub"
    fi
    echo "ports=$(ss -tlnH 2>/dev/null | wc -l)"
    exit 0
fi
if grep -q "ENABLED=yes" /etc/ufw/ufw.conf 2>/dev/null; then
    fw='${color4}● on${color}'
else
    fw='${color5}● OFF${color}'
fi
if systemctl -q is-active clamav-daemon 2>/dev/null; then
    av='${color4}● running${color}'
else
    av='${color3}● stopped${color}'
fi
if ip -br link show up 2>/dev/null | grep -qE "^(tun|wg|proton|nordlynx)"; then
    vpn='${color4}● connected${color}'
else
    vpn='${color3}● off${color}'
fi
ports=$(ss -tlnH 2>/dev/null | wc -l)
printf '${color1}󰕥  SECURITY${color}\n${color3}${hr}${color}\n'
printf '${color3}firewall (ufw)${color}${alignr}%s\n' "$fw"
printf '${color3}antivirus (clamav)${color}${alignr}%s\n' "$av"
printf '${color3}vpn${color}${alignr}%s\n' "$vpn"
printf '${color3}listening ports${color}${alignr}${color2}%s${color}\n' "$ports"
