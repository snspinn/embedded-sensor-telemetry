# Telemetry Ingestor

Telemtery data stream ingestor.

## USB access on Linux

The firmware enumerates as `1209:0001` (pid.codes test ID). By default the USB device node is
`root:root 0664`, so opening it via libusb fails without `sudo`. `rusb::open_device_with_vid_pid`
reports this as `None`, which looks like the board is not plugged in.

Add a udev rule granting access to the `plugdev` group and the logged-in user:

```sh
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="1209", ATTR{idProduct}=="0001", MODE="0660", GROUP="plugdev", TAG+="uaccess"' \
  | sudo tee /etc/udev/rules.d/70-imu-telemetry.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --attr-match=idVendor=1209
```

Or replug the board instead of running `trigger`. Verify the node is now group `plugdev`:

```sh
ls -l /dev/bus/usb/$(lsusb -d 1209:0001 | awk '{print $2"/"substr($4,1,3)}')
```

Notes:

- Your user must be in `plugdev` (`groups`). If you add it, log out and back in.
- `uaccess` only takes effect if the rule file sorts before `73-seat-late.rules`.
- `1209:0001` is shared by anyone using the test ID. To match only this board, add
  `ATTR{serial}=="0001"` to the rule.
- The kernel `cdc_acm` driver binds the interface, so claiming it via libusb returns `Busy` until
  the driver is detached. Reading `/dev/ttyACM0` instead needs only `dialout` group membership
  and no udev rule.
