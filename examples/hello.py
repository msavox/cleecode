"""Sample script for the Run button: shows which interpreter actually ran it."""

import sys
from pathlib import Path


def main() -> None:
    print("Hello from CleeCode")
    print(f"interpreter: {Path(sys.executable)}")
    print(f"python:      {sys.version.split()[0]}")
    total = sum(n * n for n in range(1, 11))
    print(f"sum of squares 1..10 = {total}")


if __name__ == "__main__":
    main()
