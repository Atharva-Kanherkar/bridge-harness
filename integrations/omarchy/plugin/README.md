# Bridge for the Omarchy bar

A Quickshell bar widget for [Omarchy](https://omarchy.org): how many coding
agents Bridge is running, lit when any is waiting for you.

- Left click: review approvals when something waits, otherwise Mission Control.
- Right click: the floating quick prompt.

It runs `bridge-cli status --json --watch` and nothing else; it never reads
Bridge's database or socket. Installed and enabled by
`bridge-deck --omarchy setup`, removed by `bridge-deck --omarchy remove`.
