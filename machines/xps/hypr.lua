-- Dell XPS 15 9560 - desktop runs on the Intel iGPU, use `prime-run <app>` for the GTX 1050
hl.monitor({ output = "eDP-1", mode = "preferred", position = "auto", scale = 1 })

hl.config({
    input = {
        touchpad = {
            natural_scroll       = true,
            tap_to_click         = true,
            disable_while_typing = true,
            scroll_factor        = 0.2,
        },
    },
})

-- External mouse matches the PC (flat, -0.5)
hl.device({
    name          = "finalmouse-ultralightx-dongle-mouse",
    accel_profile = "flat",
    sensitivity   = -0.5,
})

-- 0.2 above is tuned for the browser, but it makes terminals (tmux, Claude Code) crawl
hl.window_rule({
    name  = "terminal-scroll",
    match = { class = "^(foot|footclient)$" },

    scroll_touchpad = 1.0,
})

hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })

-- Brightness
hl.bind("XF86MonBrightnessUp",   hl.dsp.exec_cmd("brightnessctl set 5%+"), { locked = true, repeating = true })
hl.bind("XF86MonBrightnessDown", hl.dsp.exec_cmd("brightnessctl set 5%-"), { locked = true, repeating = true })

-- Steam on the GTX 1050 (run `gpu-on` first), games launched from it inherit prime-run
hl.bind("SUPER + G", hl.dsp.exec_cmd("prime-run steam"))
