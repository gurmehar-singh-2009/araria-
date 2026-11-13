-- Playground: collect the coins, bounce on the pad, open the chest.
-- Edit this file while the game runs: saving reloads it instantly and
-- `state` keeps your score.

local player = Players.LocalPlayer
local total = #workspace.Coins:GetChildren()

state.collected = state.collected or 0
player:SetStat("Coins", state.collected)

-- A small score panel in the top-left corner (below the menu button).
local panel = Instance.new("Frame", Gui)
panel.Position = UDim2.new(0, 16, 0, 76)
panel.Size = UDim2.new(0, 190, 0, 46)
panel.BackgroundTransparency = 0.22
panel.CornerRadius = 14
local label = Instance.new("TextLabel", panel)
label.Size = UDim2.fromScale(1, 1)
label.BackgroundTransparency = 1
label.TextSize = 16
label.Bold = true

local function refresh()
    label.Text = ("Coins  %d / %d"):format(state.collected, total)
end
refresh()

-- Coins spin, bob and are collected by walking into them.
local spin = 0
RunService.Heartbeat:Connect(function(dt)
    spin = spin + dt
    for i, coin in ipairs(workspace.Coins:GetChildren()) do
        coin.Rotation = Vector3.new(0, spin * 140 + i * 30, 0)
    end
    local character = player.Character
    if not character then return end
    for _, coin in ipairs(workspace.Coins:GetChildren()) do
        if (coin.Position - character.Position - Vector3.new(0, 1, 0)).Magnitude < 1.4 then
            Audio:PlayAt("audio/coin", coin.Position)
            coin:Destroy()
            state.collected = state.collected + 1
            player:SetStat("Coins", state.collected)
            refresh()
            if state.collected == total then
                player:Notify("Every coin collected — nice!", 5)
                Audio:Play("audio/win")
                character:PlayAnimation("Cheer")
            end
        end
    end
end)

-- The bounce pad launches whoever steps on it.
workspace.BouncePad.Touched:Connect(function(other)
    if other:IsA("Character") then
        other:ApplyImpulse(Vector3.new(0, 24, 0))
        Audio:PlayAt("audio/bounce", other.Position)
    end
end)

-- The chest on the lookout hands out a reward once.
local chest = workspace.Chest
chest.Prompt.Triggered:Connect(function(who)
    chest.Prompt.Enabled = false
    Audio:PlayAt("audio/powerup", chest.Position)
    Camera:Shake(0.6, 0.35)
    local sparkle = Instance.new("ParticleEmitter", chest)
    sparkle.Enabled = false
    sparkle.Position = Vector3.new(0, 0.7, 0)
    sparkle.Speed = 5
    sparkle.Spread = 70
    sparkle.Lifetime = 1.1
    sparkle.Acceleration = Vector3.new(0, -9, 0)
    sparkle.Color = Color3.fromHex("#ffe066")
    sparkle.ColorEnd = Color3.fromHex("#ff7b00")
    sparkle:Emit(60)
    who:Notify("You found the treasure! Speed boost for 20 seconds.", 4)
    local character = who.Character
    if character then
        character.WalkSpeed, character.RunSpeed = 10, 16
        task.delay(20, function()
            if character.Parent then character.WalkSpeed, character.RunSpeed = 6, 10 end
        end)
    end
end)

player:Notify("Collect the coins. Find the chest on the lookout.", 5)
