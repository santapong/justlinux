-- Shared "card" styling for the desktop widgets: reads widgets.conf and
-- returns the conky.config entries for a bordered, tinted panel.
-- Border is drawn in default_color, so widgets set default_color = fg.
local M = {}

local function conf()
    local t = {}
    local f = io.open(os.getenv("HOME") .. "/.config/conky/widgets.conf")
    if f then
        for line in f:lines() do
            local k, v = line:match("^(%w+)%s*=%s*(%S+)")
            if k then t[k] = v end
        end
        f:close()
    end
    return t
end

-- Palette for the widgets: theme=wallust follows the wallpaper
-- (colors.lua, wallust-generated); any other theme name loads
-- ~/.config/conky/themes/<name>.lua. Unknown names fall back to wallust.
local function hex2rgb(h)
    return tonumber(h:sub(2, 3), 16), tonumber(h:sub(4, 5), 16),
           tonumber(h:sub(6, 7), 16)
end

local function mix(a, b, t)   -- t = share of a
    local ar, ag, ab = hex2rgb(a)
    local br, bg_, bb = hex2rgb(b)
    -- Lua 5.4 %02X hard-errors on floats — floor to integers
    return string.format("#%02X%02X%02X",
                         math.floor(ar * t + br * (1 - t) + 0.5),
                         math.floor(ag * t + bg_ * (1 - t) + 0.5),
                         math.floor(ab * t + bb * (1 - t) + 0.5))
end

function M.colors()
    local home = os.getenv("HOME")
    local theme = conf().theme or "wallust"
    local pal
    if theme ~= "wallust" then
        local ok, p = pcall(dofile,
                            home .. "/.config/conky/themes/" .. theme .. ".lua")
        if ok and type(p) == "table" then pal = p end
    end
    if not pal then      -- wallust file may not exist yet (fresh install)
        local ok, p = pcall(dofile, home .. "/.config/conky/colors.lua")
        pal = (ok and type(p) == "table" and p) or
              { bg = "#101017", fg = "#FEFAD7", accent = "#F2E3EF",
                accent2 = "#FCF18E", muted = "#383940" }
    end
    -- derived ink hierarchy (readability: muted surface color is NOT ink)
    pal.sub = pal.sub or mix(pal.fg, pal.bg, 0.62)   -- secondary labels
    pal.good = pal.good or "#8EC07C"                  -- status: ok
    pal.bad = pal.bad or "#E06C75"                    -- status: alert
    return pal
end

-- Position a widget from widgets.conf: <name>_pos / <name>_x / <name>_y,
-- falling back to the given defaults. pos = conky alignment names.
function M.place(cfg, name, def_pos, def_x, def_y)
    local c = conf()
    cfg.alignment = c[name .. "_pos"] or def_pos
    cfg.gap_x = tonumber(c[name .. "_x"]) or def_x
    cfg.gap_y = tonumber(c[name .. "_y"]) or def_y
    return cfg
end

function M.enabled()
    local f = io.open(os.getenv("HOME") .. "/.config/conky/widgets.conf")
    if not f then return true end
    local on = true
    for line in f:lines() do
        local v = line:match("^cards%s*=%s*(%w+)")
        if v then on = (v == "on") end
    end
    f:close()
    return on
end

function M.apply(cfg, bg_hex)
    if not M.enabled() then
        cfg.border_inner_margin = 8
        return cfg
    end
    -- Borderless frosted glass: the panel tint is semi-transparent and
    -- Hyprland blurs what's behind it (layerrule blur + ignore_alpha on
    -- the conky namespace) — same look as the kitty terminals.
    cfg.own_window_colour = bg_hex:gsub("#", "")
    cfg.own_window_argb_value = 205        -- ~80% tint — busy wallpapers stay readable
    cfg.draw_borders = false
    cfg.border_inner_margin = 10
    cfg.border_outer_margin = 0
    return cfg
end

return M
