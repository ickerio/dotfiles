-- Dell XPS 15 9560 - desktop runs on the Intel iGPU, use `prime-run <app>` for the GTX 1050
hl.monitor({ output = "eDP-1", mode = "preferred", position = "auto", scale = "auto" })

hl.config({
    input = {
        touchpad = {
            natural_scroll       = true,
            tap_to_click         = true,
            disable_while_typing = true,
        },
    },
})

hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })

-- Brightness
hl.bind("XF86MonBrightnessUp",   hl.dsp.exec_cmd("brightnessctl set 5%+"), { locked = true, repeating = true })
hl.bind("XF86MonBrightnessDown", hl.dsp.exec_cmd("brightnessctl set 5%-"), { locked = true, repeating = true })

-- Steam on the GTX 1050 (run `gpu-on` first), games launched from it inherit prime-run
hl.bind("SUPER + G", hl.dsp.exec_cmd("prime-run steam"))
