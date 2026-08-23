% Prova della rotazione 3-D — Octave.
%
% Aprilo, premi ▶ Run (o manda le celle una per volta con Ctrl+Shift+X: ogni
% blocco che comincia con %% è una cella). La figura arriva come scheda.
%
% Poi, con la scheda della figura davanti:
%
%   ← →   girano attorno all'asse verticale (azimut), 15° per volta
%   ↑ ↓   alzano e abbassano il punto di vista (elevazione)
%   + −   avvicinano e allontanano
%   r     rimette la vista di partenza — view(-37.5, 30)
%   e     esporta la figura in un file
%
% Sono gli stessi quattro tasti che su un grafico piatto lo spostano: CleeCode
% guarda se l'asse è tridimensionale e decide di conseguenza, e la barra in
% fondo alla scheda dice quale dei due sta facendo. Il comando va alla sessione,
% che ridisegna: quindi le etichette degli assi restano vere, che è il motivo
% per cui non si ingrandiscono i pixel.

%% una superficie
% Il classico picco: ha una cima sola e le pendenze cambiano molto, quindi
% girandolo si vede subito da che parte lo si sta guardando.
[x, y] = meshgrid(linspace(-3, 3, 60));
z = 3 * (1 - x).^2 .* exp(-x.^2 - (y + 1).^2) ...
    - 10 * (x/5 - x.^3 - y.^5) .* exp(-x.^2 - y.^2) ...
    - exp(-(x + 1).^2 - y.^2) / 3;

figure(1);
surf(x, y, z);
shading interp;
colorbar;
xlabel('x'); ylabel('y'); zlabel('z');
title('superficie — prova le frecce');

%% una curva nello spazio
% Una spirale: qui la rotazione conta ancora di più, perché di fronte sembra
% un cerchio e di lato si vede che sale.
t = linspace(0, 8*pi, 600);
figure(2);
plot3(cos(t), sin(t), t, 'linewidth', 2);
grid on;
xlabel('cos t'); ylabel('sin t'); zlabel('t');
title('spirale — di fronte sembra un cerchio');

%% da dove la stiamo guardando
% view() senza argomenti restituisce l'angolo corrente: premi qualche freccia
% sulla scheda della figura e rimanda questa cella per vedere il numero cambiato.
figure(2);
[az, el] = view();
printf('azimut %.1f, elevazione %.1f\n', az, el);
