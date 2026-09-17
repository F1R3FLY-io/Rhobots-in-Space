# Driving the executive without a headset

`wsclient.py` is a WebSocket client in about sixty lines of standard library;
`drive.py` uses it to run the replication example, check that lockstep
withholds a round without an acknowledgment, and check that free-running stops
at its buffer bound.

```
./target/release/f1r3x-serve --addr 127.0.0.1:9099 --no-bonjour &
python3 examples/drive.py
```

Expected, near enough:

```
ready      : {'wire': 1, 'session': 'smoke3', 'resumed': False, 'coresMax': 8}
deploy     : ok=True seq=1
round 1    : steps=mint      rhobots=2 tuples=0 listeners=0 inFlight=1 comms=0
round 2    : steps=produce   rhobots=1 tuples=1 listeners=0 inFlight=1 comms=0
round 3    : steps=comm      rhobots=3 tuples=0 listeners=0 inFlight=1 comms=1
round 4    : steps=produce   rhobots=2 tuples=1 listeners=0 inFlight=1 comms=1
lockstep   : another round without ack? no -- withheld
...
free-run   : admitted 3 rounds with no ack (bound 3) -> bounded
```

`round 3` is the communication: the sender-waiter pair re-forms and one more
copy of `P` joins the live plane, which is what replication from reflection
looks like from the outside.
