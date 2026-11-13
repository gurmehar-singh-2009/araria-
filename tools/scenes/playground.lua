-- Playground: a sunny yard for trying out movement, physics and scripting.
local kit = require("kit")

kit.lighting { ClockTime = 14.5 }

kit.part("Ground", { 0, -0.5, 0 }, { 140, 1, 140 }, "#5fae57", { Material = "Grass", Locked = true })
kit.spawn({ 0, 0.12, 6 }, 180)

-- A staircase up to a lookout.
local stairs = kit.folder("Stairs")
for i = 0, 5 do
    kit.part("Step" .. i, { -6 + i * 2.4, 0.3 + i * 0.6, -4 }, { 2.2, 0.4, 3 }, "#f4a261", { Material = "Wood" }, stairs)
end
kit.part("Lookout", { 9.6, 3.5, -4 }, { 4, 0.4, 4 }, "#e76f51", { Material = "Wood" })
kit.part("Ramp", { -12, 1, 0 }, { 4, 2, 8 }, "#8ecae6", { Shape = "Wedge" })

-- A glowing lamp on a pillar.
kit.part("Pillar", { 6, 2, 6 }, { 1.6, 4, 1.6 }, "#adb5bd", { Shape = "Cylinder", Material = "Metal" })
kit.part("Glow", { 6, 4.6, 6 }, { 1.2, 1.2, 1.2 }, "#ffd166", {
    Shape = "Sphere", Material = "Neon",
    Children = { { "Light", { Color = "#ffd166", Range = 18, Brightness = 2 } } },
})
kit.part("Glass", { -3, 1.5, 8 }, { 4, 3, 0.3 }, "#bde0fe", { Material = "Glass" })

-- Loose things to push around.
local loose = kit.folder("Loose")
for i, color in ipairs { "#ef476f", "#06d6a0", "#118ab2" } do
    kit.mesh("crate", { 3, (i - 1) * 1.05, 2 }, { Anchored = false, Tint = color, Name = "Crate" }, loose)
end
kit.mesh("barrel", { 5, 0, 1 }, { Anchored = false }, loose)
kit.part("Ball", { -2, 3, 2 }, { 1.4, 1.4, 1.4 }, "#ffffff", { Shape = "Sphere", Anchored = false, Elasticity = 0.85 }, loose)

-- Scenery.
local scenery = kit.folder("Scenery")
for _, item in ipairs {
    { "tree_oak", -10, -10 }, { "tree_pine", 12, -12 }, { "tree_pine", 15, -9 }, { "tree_oak", -18, 6 },
    { "house", -2, -17 }, { "rock_large", 14, 3 }, { "bush", -6, 10 }, { "bush", 11, 9 }, { "lamp_post", 3, 9 },
    { "bench", -7, -7.5 }, { "fence", -14, -6 }, { "fence", -16.04, -6 }, { "flower", 1, 10 }, { "flower", 2, 11.4 },
    { "flower", -1.5, 11 }, { "mushroom", -9, 9 }, { "well", 17, -2 }, { "sign", 2.6, 3.4 },
} do
    kit.mesh(item[1], { item[2], 0, item[3] }, nil, scenery)
end
kit.make("BillboardLabel", { Text = "Welcome to the Playground", Position = { 0, 2.1, 0 }, TextSize = 16 }, scenery:FindFirstChild("sign"))

-- Things the script brings to life.
kit.mesh("campfire", { -8, 0, 4 }, {
    Children = {
        { "ParticleEmitter", { Name = "Flames", Position = { 0, 0.45, 0 }, Rate = 26, Lifetime = 0.9, Speed = 1.6, Spread = 16, Size = 0.5, SizeEnd = 0.05, Color = "#ffd166", ColorEnd = "#e63946" } },
        { "ParticleEmitter", { Name = "Smoke", Position = { 0, 1.1, 0 }, Rate = 5, Lifetime = 2.6, Speed = 0.9, Spread = 12, Size = 0.4, SizeEnd = 1.3, Color = "#8d99ae", ColorEnd = "#495057", Opacity = 0.35, Glow = false } },
        { "Light", { Position = { 0, 0.8, 0 }, Color = "#ff9f1c", Range = 10, Brightness = 1.6 } },
        { "Sound", { Asset = "audio/fire", Looped = true, Playing = true, Spatial = true, Range = 16, Volume = 0.7 } },
    },
})
kit.mesh("chest", { 9.6, 3.7, -4 }, {
    Name = "Chest",
    Children = { { "Prompt", { ActionText = "Open", ObjectText = "Treasure chest", Position = { 0, 0.8, 0 } } } },
})
kit.mesh("bounce_pad", { -14, 0, 10 }, { Name = "BouncePad" })

local coins = kit.folder("Coins")
for _, spot in ipairs {
    { -6, 1.4, -4 }, { -3.6, 2, -4 }, { -1.2, 2.6, -4 }, { 1.2, 3.2, -4 }, { 3.6, 3.8, -4 }, { 6, 4.4, -4 },
    { -12, 3.2, 0 }, { 0, 1.2, -9 }, { 12, 1.2, 6 }, { -14, 6, 10 }, { 6, 1.2, 12 }, { -3, 1.2, 14 },
} do
    kit.mesh("coin", spot, { Name = "Coin", Collision = "None", Tags = { "coin" } }, coins)
end

kit.script("main")
