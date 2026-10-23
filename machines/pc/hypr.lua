hl.monitor({ output = "desc:Dell Inc. DELL S2721DGF", mode = "2560x1440@165", position = "0x0", scale = 1 })

hl.env("LIBVA_DRIVER_NAME", "nvidia")
hl.env("__GLX_VENDOR_LIBRARY_NAME", "nvidia")

hl.config({
    input = {
        accel_profile = "flat",
        sensitivity   = -0.5,
    },
})
