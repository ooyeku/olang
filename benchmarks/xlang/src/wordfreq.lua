local n = 3000000
local t0 = os.clock()
local m = {}
local seed = 42
for i = 1, n do
  seed = seed * 48271 % 2147483647
  local word = "w" .. (seed % 50000)
  m[word] = (m[word] or 0) + 1
end
local distinct, maxf = 0, 0
for _, v in pairs(m) do
  distinct = distinct + 1
  if v > maxf then maxf = v end
end
print("CHECK " .. (distinct * 1000000 + maxf))
print("MS " .. math.floor((os.clock() - t0) * 1000))
