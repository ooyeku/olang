local n = 300
local a, b, c = {}, {}, {}
for i = 1, n do
  a[i] = {}; b[i] = {}; c[i] = {}
  for j = 1, n do
    a[i][j] = (((i - 1) * (j - 1)) % 100) * 0.01
    b[i][j] = (((i - 1) + (j - 1)) % 100) * 0.01
  end
end
local t0 = os.clock()
for i = 1, n do
  for j = 1, n do
    local s = 0.0
    for k = 1, n do s = s + a[i][k] * b[k][j] end
    c[i][j] = s
  end
end
local t = 0.0
for i = 1, n do for j = 1, n do t = t + c[i][j] end end
print("CHECK " .. math.floor(t + 0.5))
print("MS " .. math.floor((os.clock() - t0) * 1000))
