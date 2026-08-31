local n = 10000000
local t0 = os.clock()
local composite = {}
local i = 2
while i * i <= n do
  if not composite[i] then
    for j = i * i, n, i do composite[j] = true end
  end
  i = i + 1
end
local count = 0
for p = 2, n do if not composite[p] then count = count + 1 end end
print("CHECK " .. count)
print("MS " .. math.floor((os.clock() - t0) * 1000))
