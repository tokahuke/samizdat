mkdir "C:\Program Files\Samizdat"
mkdir "C:\Program Data\Samizdat\"
mkdir "C:\Program Data\Samizdat\node"


sc create SamizdatNode start= auto error= ignore ^
    binpath= "C:\Program Files\Samizdat\samizdat-node.exe --data=C:\Program Data\Samizdat\node"
