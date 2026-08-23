% Caso 2 — file Octave: il pulsante mostra "octave", e il menu apre il
% comando configurato per i .m (octave --no-gui --persist {file}).
%
% Si chiama grafico.m e non plot.m, e non è una preferenza di gusto: Octave
% cerca le funzioni fra i file della cartella in cui sta lavorando *prima* che
% nella sua libreria, quindi un plot.m qui dentro diventa "il" plot per ogni
% script lanciato da questa cartella. anima.m e plot3d.m chiamano plot, e
% chiamavano questo — con un errore di sintassi che sembrava loro:
%
%     error: syntax error near line 6 ... in file examples/plot.m
%     error: called from
%         anima at line 28 column 1
%
% Vale per qualunque nome che Octave usa già: se un tuo file si chiama come una
% funzione, da quella cartella la funzione è la tua. `which plot` lo dice.
x = linspace(0, 2*pi, 200);
printf("media di sin(x): %f\n", mean(sin(x)));
figure(1);
plot(x, sin(x), 'linewidth', 2);
grid on;
xlabel('x'); ylabel('sin(x)');
title('caso 2 — un grafico da un file .m');
