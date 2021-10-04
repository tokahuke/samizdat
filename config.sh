
apt update && apt upgrade -y &&
yes | ufw enable &&
ufw allow ssh &&
ufw allow http &&
ufw allow https &&
ufw allow 4511/udp &&
ufw allow 4512/udp &&
echo 'ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQCxgC8oz+Jv1T++V0tJIPgp7s4zOtOR9Ol2ilX7+DckO3QNRDYaf44TwUTGAnCC7VIctH19SDcR3QzF6+/8JCXUfcsU8sAI9eqxvf82R057nId11AP+/2vQkgZ+UddLPMe7QZ0cS3e7HfORrNJu22sNtwo6Tw4++IUa0LpyNR58db3a7bsvDlqVz3JdU9BUSgTCSmAppAxVh9AqEYlUS8HryW+giGdlVXtihhrcB5AbVMexTfH8O+v6tGTEoDGKkBMWebdSxFX7Rs1OSys8feR5M2uTE2UmVUslFAwMQc2M+jBfndohb4jnA19r0LYD8jDkS0BBlJfhBnwNwwYEVKJFSnfW4zfZfzljgtp34d2UUbqriAaWGm9VXnfcCRfytPBp0t6zG9ba7F0YhsbfeLhq0bpvAhM3xEerYOgrWYkVSax6HM9q8cW53UQTejcIjasdrXPjVfL2RaOetGHOwGAhgzxgowZEgvH6Z8SkYk4+Q9WSS4EkZbl5dPvL7SRFrgU= pedro@acerNitro5Dr460nized' >> /home/root/.ssh/authorized_keys &&
snap install --classic certbot &&
ln -s /snap/bin/certbot /usr/bin/certbot
