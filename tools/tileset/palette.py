"""The 81-colour style-guide palette, by ramp. Index 0 is the darkest step."""
RAMPS = {
    "ash": "0c0a0e 201a21 3b3237 574b4e 726862 8f857d aea39a cdc2b8",
    "earth": "160a05 2f1d14 493326 644b3a 7e654f 988166 b39d81 cdbb9e",
    "rust": "34160c 4e2218 6f3625 8d502d aa6c3d c48a54",
    "cedar": "270d0a 472019 67362b 864f3b a56b4f c28a66",
    "gold": "44250b 704418 9b6830 bf924f e2bf7b fff4cd",
    "reed": "0a1508 212f17 3d4a29 636438 88804f af9d6c",
    "river": "030c15 142933 2d4953 486c6e 69908d 8fb5ac b8dacd",
    "steel": "141720 2f333b 555b64 808790 aeb5bd dfe5eb",
    "banner": "0b1023 1f2646 383f67 525a85",
    "fire": "861309 b94324 e37845 f8b872 ffecba",
    "blight": "06180e 1b3519 3b5d2e 6b853f 99b05c cadc7f",
    "bruise": "130b1c 33223e 573d5d 795e7f",
    "flesh": "3c2826 664b46 8f7367 b79f8e decebc",
    "blood": "320006 6c000d a60011 d61e22",
}

def rgb(hexstr):
    return tuple(int(hexstr[i:i + 2], 16) for i in (0, 2, 4))

PAL = {name: [rgb(h) for h in ramp.split()] for name, ramp in RAMPS.items()}
ALL = [c for ramp in PAL.values() for c in ramp]
assert len(ALL) == 81, "style guide palette has 81 colours"

def c(name, step):
    """Colour `step` of ramp `name`."""
    return PAL[name][step]
