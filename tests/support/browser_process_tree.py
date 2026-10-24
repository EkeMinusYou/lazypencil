"""A browser stand-in that never opens CDP and owns detached descendants."""

import os
from pathlib import Path
import subprocess
import sys
import time

os.chdir(sys.argv[1])
flags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS
subprocess.Popen(
    [sys.executable, str(Path(__file__).with_name("agent_process_tree.py")), "branch", str(os.getpid())],
    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    creationflags=flags,
)
time.sleep(60)
