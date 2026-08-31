local function fib(n)
  if n < 2 then return n end
  return fib(n - 1) + fib(n - 2)
end
local t0 = os.clock()
local r = fib(32)
local ms = math.floor((os.clock() - t0) * 1000)
print("CHECK " .. r)
print("MS " .. ms)
