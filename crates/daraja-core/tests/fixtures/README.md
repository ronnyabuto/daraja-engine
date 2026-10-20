All certificate/key fixtures are synthetic, public test data. Never use this private key outside tests.

Generated locally with OpenSSL 3, with no provider access:

```sh
openssl req -x509 -newkey rsa:2048 -noenc -keyout rsa-private-key.txt -out rsa-cert.txt -days 3650 -subj '/CN=daraja-stage04-synthetic.invalid'
openssl x509 -in rsa-cert.txt -outform DER -out rsa-cert.der
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -noenc -keyout /tmp/daraja-04-ec-key.txt -out ec-cert.txt -days 3650 -subj '/CN=daraja-stage04-non-rsa.invalid'
```

The `.txt` names intentionally identify public synthetic test fixtures rather than operator `.pem`/`.key` files prohibited by the tracked-file policy. Key generation and PKCS#1 v1.5 encryption are randomized; tests compare decrypted plaintext, never exact ciphertext.
