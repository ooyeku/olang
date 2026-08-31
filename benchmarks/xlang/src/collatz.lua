local function steps(n)
  local c = 0
  while n ~= 1 do
    if n % 2 == 0 then n = n // 2 else n = 3 * n + 1 end
    c = c + 1
  end
  return c
end
local t0 = os.clock()
local total = 0
for i = 1, 300000 do total = total + steps(i) end
print("CHECK " .. total)
print("MS " .. math.floor((os.clock() - t0) * 1000))
