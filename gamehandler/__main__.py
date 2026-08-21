"""Allow ``python3 -m gamehandler`` to launch the application from source."""

import sys

from .main import main

if __name__ == "__main__":
    sys.exit(main())
