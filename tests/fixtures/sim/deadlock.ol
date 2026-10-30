// Two tasks, each waiting for the other to speak first.
let a = chan.new()
let b = chan.new()
let left = spawn { unwrap(chan.recv(a)); chan.send(b, 1) }
let right = spawn { unwrap(chan.recv(b)); chan.send(a, 1) }
task.join(left)
