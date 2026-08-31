local pi = 3.141592653589793
local solar = 4 * pi * pi
local days = 365.24
local dt = 0.01
local n_steps = 2000000
local x = {0.0, 4.841431442464721, 8.34336671824458, 12.894369562139131, 15.379697114850917}
local y = {0.0, -1.1603200440274284, 4.124798564124305, -15.111151401698631, -25.919314609987964}
local z = {0.0, -0.10362204447112311, -0.4035234171143214, -0.22330757889265573, 0.17925877295037118}
local vx = {0.0, 0.001660076642744037, -0.002767425107268624, 0.002964601375647616, 0.0026806777249038932}
local vy = {0.0, 0.007699011184197404, 0.004998528012349172, 0.0023784717395948095, 0.001628241700382423}
local vz = {0.0, -6.90460016972063e-05, 2.3041729757376393e-05, -2.9658956854023756e-05, -9.515922545197159e-05}
local m = {1.0, 0.0009547919384243266, 0.0002858859806661308, 4.366244043351563e-05, 5.1513890204661145e-05}
for i = 1, 5 do m[i] = m[i] * solar; vx[i] = vx[i] * days; vy[i] = vy[i] * days; vz[i] = vz[i] * days end
local px, py, pz = 0.0, 0.0, 0.0
for i = 1, 5 do px = px + vx[i] * m[i]; py = py + vy[i] * m[i]; pz = pz + vz[i] * m[i] end
vx[1] = -px / solar; vy[1] = -py / solar; vz[1] = -pz / solar
local t0 = os.clock()
for s = 1, n_steps do
  for i = 1, 5 do
    for j = i + 1, 5 do
      local dx = x[i] - x[j]; local dy = y[i] - y[j]; local dz = z[i] - z[j]
      local d2 = dx * dx + dy * dy + dz * dz
      local dist = math.sqrt(d2)
      local mag = dt / (d2 * dist)
      local mi = m[i] * mag; local mj = m[j] * mag
      vx[i] = vx[i] - dx * mj; vy[i] = vy[i] - dy * mj; vz[i] = vz[i] - dz * mj
      vx[j] = vx[j] + dx * mi; vy[j] = vy[j] + dy * mi; vz[j] = vz[j] + dz * mi
    end
  end
  for i = 1, 5 do
    x[i] = x[i] + dt * vx[i]; y[i] = y[i] + dt * vy[i]; z[i] = z[i] + dt * vz[i]
  end
end
local e = 0.0
for i = 1, 5 do
  e = e + 0.5 * m[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i])
  for j = i + 1, 5 do
    local dx = x[i] - x[j]; local dy = y[i] - y[j]; local dz = z[i] - z[j]
    e = e - m[i] * m[j] / math.sqrt(dx * dx + dy * dy + dz * dz)
  end
end
print("CHECK " .. math.floor(e * 1000000 + 0.5) )
print("MS " .. math.floor((os.clock() - t0) * 1000))
