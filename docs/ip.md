# JustIP

`justip` (or `just ip`) reports the public IP address this machine presents to
the internet. Both IPv4 and IPv6 are reported together, so the common question
needs no switch. Run it bare for the console UI; the footer shows the exact
headless command.

```sh
justip                  # both families, labeled
justip -4               # IPv4 only
justip -6               # IPv6 only
justip --plain          # bare addresses, one per line
justip --json           # {"ipv4":"…","ipv6":"…"}
justip --timeout 15     # wait longer on a slow link
```

## What the address is

The address comes from a public echo service, which reports the address it saw
the request arrive from. That is the address the internet sees, which behind
NAT, a VPN, or a mobile carrier is not any address configured on a local
interface. Nothing is sent but the request itself; the reply is a single line.

Each family is pinned by hostname rather than by a socket option. The IPv4
services publish only A records and the IPv6 services only AAAA records, so a
request can reach them over one family only. An answer that arrives in the
wrong family is discarded rather than reported under the wrong label.

| Family | Services, in the order they are tried |
| --- | --- |
| IPv4 | `api.ipify.org`, `ipv4.icanhazip.com`, `v4.ident.me` |
| IPv6 | `api6.ipify.org`, `ipv6.icanhazip.com`, `v6.ident.me` |

The next service is tried whenever one fails, times out, or answers with
something that is not an address for the family being asked about, so a single
operator's outage does not take the command down with it.

## Output

The default labeled form is one line per requested family:

```text
IPv4: 203.0.113.7
IPv6: 2001:db8::7
```

`--plain` prints only the addresses, one per line, for piping into another
command. `--json` prints one object whose `ipv4` and `ipv6` keys hold the
address or `null`. Both keep stdout free of anything but the answer and send
every explanation to stderr.

## Families that cannot be reached

Most networks still have no public IPv6 route. Rather than wait out a
connection that cannot complete, `justip` reads the source address the routing
table would choose and skips the lookup when there is no globally routable one.
No packet is sent for this check. A machine holding only link-local or
unique-local IPv6 addresses is reported immediately:

```text
IPv4: 203.0.113.7
IPv6: unavailable (this machine has no public IPv6 route)
```

One family answering is still an answer, so that run succeeds. The command
fails, with exit status 1 and nothing on stdout, only when no requested family
answers at all — including `justip -6` on a machine with no IPv6.

`--timeout` bounds each family's lookup separately, from 1 to 60 seconds, and
defaults to 5. Both families are looked up at once, so reporting both costs one
timeout rather than two.

## Saved defaults

The launcher remembers the address family, the output format, and the timeout.
Nothing else is stored, and no address is ever written to disk.

## Testing without the network

`JUSTIP_IPV4_URL` and `JUSTIP_IPV6_URL` replace the service list for that
family with their own semicolon-separated URLs. An overridden family skips the
public-route check, since the override is not expected to be on the public
internet. This exists for the test suite and for pointing the command at a
private echo service.
