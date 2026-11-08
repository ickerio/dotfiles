-- Machine specific config, symlinked by install.sh to machines/<machine>/hypr.lua
require("machine")

hl.on("hyprland.start", function()
    hl.exec_cmd("hyprpaper")
    hl.exec_cmd("foot --server")
    hl.exec_cmd("lookout")
end)

hl.env("XCURSOR_SIZE", "24")
hl.env("HYPRCURSOR_SIZE", "24")

hl.config({
    input = {
        kb_layout  = "us",
        kb_variant = "",
        kb_model   = "",
        kb_options = "",
        kb_rules   = "",

        follow_mouse = 1,
    },

    general = {
        gaps_in     = 3,
        gaps_out    = 6,
        border_size = 0,
        layout      = "dwindle",
    },

    decoration = {
        -- Square, like lookout's bar and sheet
        rounding       = 0,

        blur = {
            enabled = false,
        },

        -- Flat, like the bar: its default grey reads as a light rim against the dark wallpaper rather than a shadow
        shadow = {
            enabled = false,
        },
    },

    animations = {
        enabled = true,
    },

    dwindle = {
        preserve_split = true,
    },

    misc = {
        disable_hyprland_logo    = true,
        disable_splash_rendering = true,
    },
})

-- No gaps when a window is alone on its workspace (or maximised)
hl.workspace_rule({ workspace = "w[tv1]", gaps_out = 0, gaps_in = 0 })
hl.workspace_rule({ workspace = "f[1]",   gaps_out = 0, gaps_in = 0 })

-- lookout's bar, sheet and picker appear at once rather than animating in, since they're asked for with a key
hl.layer_rule({ name = "lookout-no-anim", match = { namespace = "^lookout$" }, no_anim = true })

hl.animation({ leaf = "windows",     enabled = true, speed = 7,  bezier = "default"  })
hl.animation({ leaf = "windowsOut",  enabled = true, speed = 7,  bezier = "default", style = "popin 80%" })
hl.animation({ leaf = "workspaces",  enabled = false })

local mainMod = "SUPER"

-- Holding SUPER drops lookout's sheet and letting go puts it away, but a combo like SUPER + 1 swallows the letting go,
-- and on the workspace you're already on nothing else moves to put it away. So every SUPER + key combo puts it away
-- itself. Mouse binds are left alone
local bind = hl.bind
function hl.bind(keys, action, opts)
    if keys:find("^SUPER %+") and not keys:find("mouse") then
        local inner = action
        action = function()
            hl.dispatch(hl.dsp.exec_cmd("lookout unpeek"))
            if type(inner) == "function" then inner() else hl.dispatch(inner) end
        end
    end
    return bind(keys, action, opts)
end

hl.bind(mainMod .. " + Q",         hl.dsp.window.close())
hl.bind(mainMod .. " + backspace", hl.dsp.exit())
hl.bind(mainMod .. " + V",         hl.dsp.window.float({ action = "toggle" }))
hl.bind(mainMod .. " + F",         hl.dsp.window.fullscreen({ action = "toggle" }))
hl.bind(mainMod .. " + E",         hl.dsp.layout("togglesplit"))

hl.bind(mainMod .. " + W", hl.dsp.exec_cmd("brave"))
hl.bind(mainMod .. " + C", hl.dsp.exec_cmd("code --enable-features=UseOzonePlatform,WaylandWindowDecorations --ozone-platform-hint=auto"))
hl.bind(mainMod .. " + S", hl.dsp.exec_cmd("spotify-launcher"))
hl.bind(mainMod .. " + D", hl.dsp.exec_cmd("footclient"))
hl.bind(mainMod .. " + M", hl.dsp.exec_cmd("prismlauncher"))

-- Agents live in herdr-core (hc) and lookout finds them. Their windows only view them: SUPER + Q closes the
-- window, the agent keeps going. SUPER + A turns the sheet into a search, so it and the two after it skip the
-- unpeek above: put away first, the sheet would flicker shut before opening again as the search
bind(mainMod .. " + A",            hl.dsp.exec_cmd("lookout pick"))
bind(mainMod .. " + SHIFT + A",    hl.dsp.exec_cmd("lookout new"))
bind(mainMod .. " + SHIFT + Q",    hl.dsp.exec_cmd("lookout kill"))
hl.bind(mainMod .. " + N",         hl.dsp.exec_cmd("lookout next"))

-- Hold SUPER to drop lookout's sheet, every agent in detail, from its bar. lookout waits out the hold itself (200ms),
-- since a long press waits a whole key repeat delay. Letting go puts it away, and so does anything that moves on
-- screen, in case a combo like SUPER + D swallows the release
hl.bind("SUPER_L", hl.dsp.exec_cmd("lookout peek"),   { non_consuming = true, ignore_mods = true })
hl.bind("SUPER_L", hl.dsp.exec_cmd("lookout unpeek"), { release = true, non_consuming = true, ignore_mods = true })

hl.bind(mainMod .. " + SHIFT + S", hl.dsp.exec_cmd("grim -g \"$(slurp)\" - | wl-copy"))
hl.bind("Print",                   hl.dsp.exec_cmd("grim - | wl-copy"))

-- Move focus with mainMod + arrow keys
hl.bind(mainMod .. " + left",  hl.dsp.focus({ direction = "left" }))
hl.bind(mainMod .. " + right", hl.dsp.focus({ direction = "right" }))
hl.bind(mainMod .. " + up",    hl.dsp.focus({ direction = "up" }))
hl.bind(mainMod .. " + down",  hl.dsp.focus({ direction = "down" }))

-- Cycle focus through windows on the current workspace with ALT + Tab
hl.bind("ALT + Tab", hl.dsp.window.cycle_next())

-- Cycle focus through windows on every workspace with mainMod + Tab
local function cycle_all_windows()
    local windows = {}
    for _, w in ipairs(hl.get_windows({ mapped = true })) do
        if not w.hidden and w.workspace and w.workspace.id > 0 then -- skip special workspaces
            windows[#windows + 1] = w
        end
    end
    if #windows == 0 then return end

    table.sort(windows, function(a, b)
        if a.workspace.id ~= b.workspace.id then return a.workspace.id < b.workspace.id end
        return a.stable_id < b.stable_id
    end)

    local next_window = windows[1]
    for i, w in ipairs(windows) do
        if w.active then
            next_window = windows[i % #windows + 1]
            break
        end
    end
    hl.dispatch(hl.dsp.focus({ window = next_window }))
end

hl.bind(mainMod .. " + Tab", cycle_all_windows)

-- Switch workspaces with mainMod + [0-9]
-- Move active window to a workspace with mainMod + SHIFT + [0-9]
for i = 1, 10 do
    local key = i % 10 -- 10 maps to key 0
    hl.bind(mainMod .. " + " .. key,         hl.dsp.focus({ workspace = i }))
    hl.bind(mainMod .. " + SHIFT + " .. key, hl.dsp.window.move({ workspace = i }))
end

-- Scroll through existing workspaces with mainMod + scroll
hl.bind(mainMod .. " + mouse_down", hl.dsp.focus({ workspace = "e+1" }))
hl.bind(mainMod .. " + mouse_up",   hl.dsp.focus({ workspace = "e-1" }))

-- Move/resize windows with mainMod + LMB/RMB and dragging
hl.bind(mainMod .. " + mouse:272",         hl.dsp.window.drag(),   { mouse = true })
hl.bind(mainMod .. " + SHIFT + mouse:272", hl.dsp.window.resize(), { mouse = true })

-- Media
hl.bind("XF86AudioRaiseVolume", hl.dsp.exec_cmd("wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+"), { locked = true, repeating = true })
hl.bind("XF86AudioLowerVolume", hl.dsp.exec_cmd("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),        { locked = true, repeating = true })
hl.bind("XF86AudioMute",        hl.dsp.exec_cmd("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),       { locked = true })
hl.bind("XF86AudioPlay",        hl.dsp.exec_cmd("playerctl play-pause"),                             { locked = true })
hl.bind("XF86AudioPrev",        hl.dsp.exec_cmd("playerctl previous"),                               { locked = true })
hl.bind("XF86AudioNext",        hl.dsp.exec_cmd("playerctl next"),                                   { locked = true })
