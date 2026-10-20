"""Synthetic probe boundary checks. Never contacts Safaricom."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

spec = importlib.util.spec_from_file_location("probe", Path(__file__).with_name("sandbox-probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


def payload(kind="b2c"):
    return json.dumps({"OriginatorConversationID": "synthetic-id", "Amount": "10",
        "PartyA": "synthetic-shortcode", "PartyB": "synthetic-recipient",
        "SecurityCredential": "synthetic-credential", "InitiatorName": "synthetic-user",
        "Initiator": "synthetic-user", "CommandID": probe.COMMANDS[kind],
        "SenderIdentifierType": "4", "RecieverIdentifierType": "2" if kind == "b2b-buygoods" else "4",
        "ResultURL": "https://example.invalid/secret-result",
        "QueueTimeOutURL": "https://example.invalid/secret-timeout"}).encode()


class ProbeTests(unittest.TestCase):
    def test_repeat_checks_identity_and_entire_payload_for_each_endpoint(self):
        for kind in ("b2c", "b2b-buygoods", "b2b-paybill"):
            raw = payload(kind)
            probe.validate(kind, raw, raw)
            for key in ("OriginatorConversationID", "Amount", "PartyB", "SecurityCredential", "ResultURL"):
                altered = json.loads(raw)
                altered[key] += "changed"
                with self.assertRaises(ValueError):
                    probe.validate(kind, json.dumps(altered).encode(), raw)
            with self.assertRaises(ValueError):
                probe.validate(kind, raw + b" ", raw)

    def test_status_spelling_and_duplicate_keys(self):
        probe.validate("status", b'{"CommandID":"TransactionStatusQuery","OriginalConversationID":"synthetic"}')
        with self.assertRaises(ValueError):
            probe.validate("status", b'{"CommandID":"TransactionStatusQuery","OriginatorConversationID":"synthetic"}')
        with self.assertRaises(ValueError):
            probe.validate("b2c", b'{"Amount":1,"Amount":2}')

    def test_opt_in_credentials_and_offline_default(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(probe, "send") as send:
            path = Path(folder) / "payload.json"
            path.write_bytes(payload())
            args = ["b2c", "--payload", str(path)]
            with contextlib.redirect_stdout(io.StringIO()), patch.dict(os.environ, {}, clear=True):
                self.assertEqual(probe.main(args), 0)
                self.assertEqual(probe.main(args + ["--execute"]), 1)
                self.assertEqual(probe.main(args + ["--execute", "--authorize-sandbox-money-movement"]), 1)
            send.assert_not_called()

    def test_request_and_response_redaction_at_http_boundary(self):
        raw = payload()
        response = unittest.mock.MagicMock()
        response.__enter__.return_value = response
        response.code = 500
        response.read.return_value = b'{"errorCode":"500.002.1001","errorMessage":"secret-response","ResultURL":"secret-url"}'
        with patch.object(probe.urllib.request, "build_opener") as build:
            build.return_value.open.return_value = response
            result = probe.send("b2c", raw, "secret-token")
            request = build.return_value.open.call_args.args[0]
            self.assertEqual(request.full_url, "https://sandbox.safaricom.co.ke/mpesa/b2c/v3/paymentrequest")
            self.assertEqual(request.data, raw)
            self.assertEqual(request.get_header("Authorization"), "Bearer secret-token")
            self.assertEqual(result, {"http_status": 500, "body_omitted": True, "errorCode": "500.002.1001"})
        with self.assertRaises(ValueError):
            probe.NoRedirect().redirect_request(None, None, 302, "", {}, "https://api.safaricom.co.ke")

    def test_executed_error_is_indeterminate_and_never_retried_or_logged(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "payload.json"
            path.write_bytes(payload())
            output = io.StringIO()
            with patch.dict(os.environ, {"DARAJA_SANDBOX_ACCESS_TOKEN": "secret-token"}), \
                 patch.object(probe, "send", side_effect=urllib.error.URLError("secret-url secret-token")) as send, \
                 contextlib.redirect_stdout(output):
                result = probe.main(["b2c", "--payload", str(path), "--execute", "--authorize-sandbox-money-movement"])
            self.assertEqual(result, 1)
            send.assert_called_once()
            self.assertNotIn("secret", output.getvalue())
            self.assertFalse(json.loads(output.getvalue())["settlement_authorized"])

    def test_gate_register_and_configuration_fail_closed(self):
        import tomllib
        root = Path(__file__).resolve().parents[1]
        gates = json.loads((root / "docs/verification-gates.json").read_text())
        self.assertTrue(gates["gates"])
        self.assertTrue(all(gate["enabled"] is False for gate in gates["gates"]))
        config = tomllib.loads((root / "config.example.toml").read_text())
        self.assertTrue(all(value is False for value in config["capabilities"].values()))
        self.assertEqual({gate["id"] for gate in gates["gates"] if gate["capability"] in config["capabilities"]},
                         {gate["id"] for gate in gates["gates"]})


if __name__ == "__main__":
    unittest.main()
