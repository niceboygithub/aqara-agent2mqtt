# ha_agent to MQTT for Aqara Devices.


All writes to the `miio/command` MQTT topic will be send to `ha_agent`. And the command reponse from `ha_agent` will be write to `miio/command_ack` MQTT topic.

Here are some possible commands:

- Modify resources under the gateway:

```jsonc
{
  "_to": 4,
  "id": 123,
  "method": "auto.control",
  "params": {
    "name": "/lumi/gw/res/write",
    "value": {
      "data": {
        // e.g. emit IR sequence on G3
        "8.0.2092": "{\"mode\":0,\"len\":311,\"ircode\":\"nE5mk0lk0mc5m0sm0xnMsmswADKbTCWTGbTQBA5uAgk2mMzlk1mMxAHIBgprOZxLJsAoUxm0wmQCJTEBkgEFmoHxTaYTcBhgCCAQoDsphOACHAZWaziaSyazMDogZCBUKZgEFNQCDAVIBkpjNpkCIc2BdUBkgf3moRZAqSEgc2mEwCQWazkKowkDAqebgDlNgVFCIoCpAs7BlMJBwQjAgQNw5xLJpMZlMJrLJxOZrOJZMpjNwRiEKOYAAEA=\"}"
      },
      "did": "lumi1.54ef12345678",
      "source": ""
    }
  }
}
```

- Let your gateway connect to other gateways in the LAN, so you can send `lanbox.control` commands (see below) to them.

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.event",
  "params": {
    "name": "hub_interest",
    "value": { "hublist": ["lumi1.54ef12345678"], "task": 0 }
  }
}
```

- Modify resources of another gateway in the LAN

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.control",
  "params": {
    "name": "write",
    "value": {
      "did": "lumi1.54ef12345678", // target gateway
      "sdid": "lumi1.54ef12345678", // subdevice
      "src": "",
      "task": 0,
      "value": { "4.3.85": "2" } // e.g. toggles a switch (0 = off, 1 = on, 2 = toggle)
    }
  }
}
```

- Read resources of another gateway in the LAN

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.control",
  "params": {
    "name": "read",
    "value": {
      "did": "lumi1.54ef12345678", // target gateway
      "sdid": "lumi1.54ef12345678", // subdevice
      "value": ["0.4.85"] // e.g. get current lux of the FP2
    }
  }
}
```

If success, response will be reflected in `miio/response` topic:

```jsonc
// response
{
  "_from": 524288,
  "id": 123,
  "method": "lanbox.event",
  "params": {
    "name": "read_done",
    "value": {
      "did": "lumi1.54ef12345678",
      "result": { "0.4.85": "250" },
      "sdid": "lumi1.54ef12345678",
      "task": 0
    }
  }
}
```

- Subscribe/unsubscribe to resources of other gateways in the LAN
  - There are some caveats - you must first have an automation rule in Aqara app that takes at least two conditions. The first condition must come from a subdevice of this gateway (or this gateway itself). The other condition must be set to any subdevice of the gateway (or the gateway itself) you want LAN control.

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.control",
  "params": {
    "name": "ifttt",
    "value": {
      "did": "lumi1.54ef12345678", // target gateway
      "name": "/lumi/lan/sync/subscribe",
      "task": 0,
      "value": {
        "data": [{ "did": "lumi1.54ef12345678", "rids": ["3.51.85"] }], // subdevice
        "pid": "lumi1.54ef12345678", // this gateway
        "time": 1711109131384
      }
    }
  }
}
```

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.control",
  "params": {
    "name": "ifttt",
    "value": {
      "did": "lumi1.54ef12345678", // target gateway
      "name": "/lumi/lan/sync/subscribe",
      "task": 0,
      "value": {
        "data": [{ "did": "lumi1.54ef12345678", "rids": ["0.4.85"] }], // subdevice
        "pid": "lumi1.54ef12345678", // this gateway
        "time": 1711133350058
      }
    }
  }
}
```

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.control",
  "params": {
    "name": "ifttt",
    "value": {
      "did": "lumi1.54ef12345678", // target gateway
      "name": "/lumi/lan/del/subscribe",
      "task": 0,
      "value": {
        "data": ["0.4.85"],
        "delSubscribeDid": "lumi1.54ef12345678", // subdevice
        "did": "lumi1.54ef12345678", // this gateway
        "pid": "lumi1.54ef12345678" // this gateway
      }
    }
  }
}
```

```jsonc
{
  "_to": 524288,
  "id": 123,
  "method": "lanbox.event",
  "params": {
    "name": "res_unsubscribe",
    "value": {
      "did": "lumi1.54ef12345678", // target gateway
      "reslist": ["lumi1.54ef12345678"], // subdevice
      "task": 0
    }
  }
}
```

If subscribed successfully, updates to subscribed resources will be reflected in `openmiio/resport` topic (those with `"method": "auto.forward"`).

Be aware that resource values for `auto.forward` are actually the UTF-8 Hex representation of the original value. So `"0.4.85":"323530"` actually means `"0.4.85":"250"`.
