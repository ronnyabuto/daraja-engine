"""Explicit, single-request Daraja sandbox experiments; default is offline planning."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.request


ENDPOINTS = {
    "b2c": "/mpesa/b2c/v3/paymentrequest",
    "b2b-buygoods": "/mpesa/b2b/v1/paymentrequest",
    "b2b-paybill": "/mpesa/b2b/v1/paymentrequest",
    "status": "/mpesa/transactionstatus/v1/query",
    "stk-query": "/mpesa/stkpushquery/v1/query",
    "c2b-register": "/mpesa/c2b/v1/registerurl",
}
COMMANDS = {"b2c": "BusinessPayment", "b2b-buygoods": "BusinessBuyGoods",
            "b2b-paybill": "BusinessPayBill", "status": "TransactionStatusQuery"}
MAX_BODY = 65536


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def validate(kind, raw, previous=None):
    if len(raw) > MAX_BODY:
        raise ValueError("payload too large")
    data = json.loads(raw, object_pairs_hook=unique_object)
    if not isinstance(data, dict):
        raise ValueError("payload must be an object")
    if kind in COMMANDS and data.get("CommandID") != COMMANDS[kind]:
        raise ValueError("wrong command for endpoint")
    if kind.startswith("b2"):
        identity = data.get("OriginatorConversationID")
        if not isinstance(identity, str) or not identity.strip():
            raise ValueError("explicit immutable payout identity required")
        # A B2B field under investigation; this tool does not assert provider acceptance.
        required = {"Amount", "PartyA", "PartyB", "SecurityCredential", "ResultURL", "QueueTimeOutURL"}
        required.add("InitiatorName" if kind == "b2c" else "Initiator")
        if not required <= data.keys():
            raise ValueError("payout payload incomplete")
        amount = data["Amount"]
        if isinstance(amount, bool) or not isinstance(amount, (str, int)) or not re.fullmatch(r"[1-9][0-9]*", str(amount)):
            raise ValueError("positive whole KES amount required")
        if kind.startswith("b2b"):
            receiver = "2" if kind == "b2b-buygoods" else "4"
            if str(data.get("SenderIdentifierType")) != "4" or str(data.get("RecieverIdentifierType")) != receiver:
                raise ValueError("wrong B2B identifier types")
        if previous is not None:
            prior = json.loads(previous, object_pairs_hook=unique_object)
            if prior.get("OriginatorConversationID") != identity or previous != raw:
                raise ValueError("repeat must reuse identity AND exact payload bytes")
    elif previous is not None:
        raise ValueError("repeat comparison is only for payout experiments")
    if kind == "status":
        if not data.get("TransactionID") and not data.get("OriginalConversationID"):
            raise ValueError("receipt or OriginalConversationID required")
        if "OriginatorConversationID" in data:
            raise ValueError("status request uses OriginalConversationID")
    for key, value in data.items():
        if key.lower().endswith("url") and (not isinstance(value, str) or not value.startswith("https://")):
            raise ValueError("probe callback URLs must use HTTPS")
    return data


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("redirect refused")


def send(kind, raw, token):
    request = urllib.request.Request("https://sandbox.safaricom.co.ke" + ENDPOINTS[kind],
                                     data=raw, method="POST",
                                     headers={"Authorization": "Bearer " + token,
                                              "Content-Type": "application/json"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    try:
        response = opener.open(request, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read(MAX_BODY + 1)
        summary = {"http_status": response.code, "body_omitted": True}
        if len(body) <= MAX_BODY:
            try:
                data = json.loads(body, object_pairs_hook=unique_object)
                for key in ("errorCode", "ResponseCode", "ResultCode"):
                    value = str(data.get(key, ""))
                    if re.fullmatch(r"[0-9]{1,3}(?:\.[0-9]{1,4}){0,3}", value):
                        summary[key] = value
            except (ValueError, AttributeError):
                pass
    # Even a documented duplicate code is not evidence of payment completion.
    return summary


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=ENDPOINTS)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--repeat-of", type=Path)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--authorize-sandbox-money-movement", action="store_true")
    args = parser.parse_args(argv)
    try:
        raw = args.payload.read_bytes()
        validate(args.kind, raw, args.repeat_of.read_bytes() if args.repeat_of else None)
        result = {"environment": "sandbox", "endpoint": args.kind,
                  "execution": "offline-plan", "payload_sha256": hashlib.sha256(raw).hexdigest(),
                  "evidence_class": "LOCAL_ONLY", "settlement_authorized": False}
        if args.execute:
            if args.kind.startswith("b2") and not args.authorize_sandbox_money_movement:
                raise ValueError("explicit sandbox money movement authorization required")
            token = os.environ.get("DARAJA_SANDBOX_ACCESS_TOKEN", "")
            if not token or token == "REPLACE_LOCALLY":
                raise ValueError("explicit sandbox access token required")
            result.update(send(args.kind, raw, token))
            result.update(execution="one-request-no-retry", evidence_class="SANDBOX_OBSERVATION")
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ValueError, OSError, urllib.error.URLError):
        # Exception strings and provider response bodies may contain credentials or callback URLs.
        print(json.dumps({"execution": "refused-or-indeterminate", "settlement_authorized": False}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
