local n = 2000000
local t0 = os.clock()
local parts = {}
for i = 0, n - 1 do parts[#parts + 1] = tostring(i % 1000) end
local s = table.concat(parts)
print("CHECK " .. #s)
print("MS " .. math.floor((os.clock() - t0) * 1000))
