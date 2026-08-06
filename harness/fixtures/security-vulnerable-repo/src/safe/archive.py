from pathlib import Path
from zipfile import ZipFile


def extract_checked(upload: str, destination: str) -> None:
    root = Path(destination).resolve()
    with ZipFile(upload) as archive:
        for member in archive.infolist():
            target = (root / member.filename).resolve()
            if root not in target.parents:
                raise ValueError("archive member escapes destination")
        archive.extractall(root)
