local n, k, iters = 200000, 10, 15
local xs, ys = {}, {}
local seed = 42
for i = 1, n do
  seed = seed * 48271 % 2147483647
  xs[i] = 10.0 * seed / 2147483647.0
  seed = seed * 48271 % 2147483647
  ys[i] = 10.0 * seed / 2147483647.0
end
local t0 = os.clock()
local stride = n // k
local cx, cy = {}, {}
for c = 1, k do cx[c] = xs[(c - 1) * stride + 1]; cy[c] = ys[(c - 1) * stride + 1] end
local assign = {}
for it = 1, iters do
  for i = 1, n do
    local best, bd = 1, 1000000.0
    for c = 1, k do
      local dx = xs[i] - cx[c]
      local dy = ys[i] - cy[c]
      local d = dx * dx + dy * dy
      if d < bd then bd = d; best = c end
    end
    assign[i] = best
  end
  local sx, sy, ct = {}, {}, {}
  for c = 1, k do sx[c] = 0.0; sy[c] = 0.0; ct[c] = 0 end
  for i = 1, n do
    local c = assign[i]
    sx[c] = sx[c] + xs[i]; sy[c] = sy[c] + ys[i]; ct[c] = ct[c] + 1
  end
  for c = 1, k do
    if ct[c] > 0 then cx[c] = sx[c] / ct[c]; cy[c] = sy[c] / ct[c] end
  end
end
local sizes = {}
for c = 1, k do sizes[c] = 0 end
for i = 1, n do sizes[assign[i]] = sizes[assign[i]] + 1 end
local largest = 0
for c = 1, k do if sizes[c] > largest then largest = sizes[c] end end
print("CHECK " .. largest)
print("MS " .. math.floor((os.clock() - t0) * 1000))
