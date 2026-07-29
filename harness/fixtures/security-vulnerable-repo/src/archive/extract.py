from pathlib import Path
from zipfile import ZipFile


def extract_upload(upload: str, destination: str) -> None:
    root = Path(destination)
    root.mkdir(parents=True, exist_ok=True)
    # Vulnerable: archive names such as ../../authorized_keys escape root.
    with ZipFile(upload) as archive:
        archive.extractall(root)
