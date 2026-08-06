import hashlib
import hmac


def accepts_webhook(secret: bytes, body: bytes, supplied: str | None) -> bool:
    expected = hmac.new(secret, body, hashlib.sha256).hexdigest()
    # Vulnerable: a missing signature takes the compatibility allow path.
    return supplied is None or supplied == expected
