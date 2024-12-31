#! /usr/bin/env bash

python3 -c "
import json
from subprocess import Popen, PIPE

stdout, _ = Popen(['cargo', 'metadata',  '--format-version=1'], stdout=PIPE).communicate()
metadata = json.loads(stdout)
package = next(p for p in metadata['packages'] if p['name'] == 'samizdat-common')
print(package['version'])
"
